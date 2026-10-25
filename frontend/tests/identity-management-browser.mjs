/** Production UsersPage in real React/Vite; HTTP fixtures isolate persistence.
 * Run through the root gate with an explicit absolute report directory:
 * IDENTITY_MANAGEMENT_BROWSER_REPORT_DIR=/tmp/identity-management node tests/identity-management-browser.mjs
 * These checks establish browser contracts, not PostgreSQL/backend correctness.
 */
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
const reportPath = process.env.IDENTITY_MANAGEMENT_BROWSER_REPORT_DIR
assert(reportPath && path.isAbsolute(reportPath), 'Set an explicit absolute IDENTITY_MANAGEMENT_BROWSER_REPORT_DIR for screenshots/report')
const reportDirectory = path.resolve(reportPath)
for (const sourceDirectory of ['src', 'tests', 'node_modules']) {
  const source = path.join(workspace, sourceDirectory)
  const relative = path.relative(source, reportDirectory)
  assert(relative.startsWith('..') || path.isAbsolute(relative), 'Browser output must not overwrite repository source or dependencies')
}
const bindHost = process.env.IDENTITY_MANAGEMENT_BROWSER_BIND || (process.platform === 'win32' ? '::1' : '127.0.0.1')
const pause = ms => new Promise(resolve => setTimeout(resolve, ms))
const id = label => fixtureUuid(`identity-management:${label}`)
const ids = Object.fromEntries(['actor', 'user-a', 'user-b', 'tenant', 'organization', 'actor-member', 'employee', 'customer', 'system-app', 'receipt'].map(label => [label, id(label)]))
const token = 'identity-management-browser-session'
const fullCapabilities = () => ({ lifecycleEnabled: true, customerMembershipsEnabled: true, dualMembershipsEnabled: true })
const tenant = { id: ids.tenant, name: '身份验收租户', code: 'identity-fixture', isPlatform: true, status: 'ENABLED' }
const organization = { id: ids.organization, tenantId: tenant.id, parentId: null, name: '身份验收组织', code: 'identity-fixture-org',
  status: 'ENABLED', organizationType: 'COMPANY', sortOrder: 0, description: '', children: [] }
const account = (userId, username, realName) => ({ id: userId, username, realName, email: 'shared-contact@example.test', phone: '13800138000',
  gender: 'UNKNOWN', status: 'ENABLED', createdAt: '2026-10-24T00:00:00Z', lastLoginAt: null, roleCount: 1, memberships: [] })
const accounts = [account(ids['user-a'], 'same-contact-a', '同名账号甲'), account(ids['user-b'], 'same-contact-b', '同名账号乙'), account(ids.actor, 'fixture-admin', '验收管理员')]
const member = (memberId, userId, memberType, status = 'ENABLED', identityVersion = 7) => ({
  id: memberId, userId, organizationId: organization.id, memberType, status, identityVersion,
  departmentId: null, position: memberType === 'EMPLOYEE' ? '客服经理' : '', updatedAt: '2026-10-24T12:00:00Z',
})
const initialMembers = () => [member(ids.employee, ids['user-a'], 'EMPLOYEE'), member(ids.customer, ids['user-a'], 'CUSTOMER', 'DISABLED', 12), member(ids['actor-member'], ids.actor, 'EMPLOYEE', 'ENABLED', 3)]
const profile = profileWithIdentity({
  user: { ...accounts[2], avatar: '', isSuperAdmin: false }, tenant,
  organization: { id: organization.id, name: organization.name, path: `/${organization.id}` }, platformContext: true,
  applications: [{ id: ids['system-app'], name: '系统管理', code: 'system', icon: '', basePath: '/system' }],
  memberships: [{ userId: ids.actor, username: 'fixture-admin', realName: '验收管理员', tenantId: tenant.id, tenantName: tenant.name,
    organizationId: organization.id, organizationName: organization.name, organizationPath: `/${organization.id}`,
    membershipId: ids['actor-member'], memberType: 'EMPLOYEE', identityVersion: '3', status: 'ENABLED', consoleSelectable: true, selectionDisabledReason: null }],
}, { membershipId: ids['actor-member'], identityVersion: '3', accountVersion: '0' })
const proof = identityForProfile(profile)
const clone = value => structuredClone(value)
let browser, vite, origin, startupOutput = ''
const scenarios = [], screenshots = []

async function freePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer().on('error', reject)
    server.listen(0, bindHost, () => { const port = server.address().port; server.close(error => error ? reject(error) : resolve(port)) })
  })
}

async function startServer() {
  const port = await freePort()
  origin = `http://${bindHost.includes(':') ? `[${bindHost}]` : bindHost}:${port}`
  vite = spawn(process.execPath, [path.join(workspace, 'node_modules/vite/bin/vite.js'), '--host', bindHost, '--port', String(port), '--strictPort'], {
    cwd: workspace, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'],
  })
  vite.stdout.on('data', chunk => { startupOutput = (startupOutput + chunk).slice(-5000) })
  vite.stderr.on('data', chunk => { startupOutput = (startupOutput + chunk).slice(-5000) })
  let spawnError
  vite.on('error', error => { spawnError = error })
  await waitState(() => {
    if (spawnError) throw spawnError
    if (vite.exitCode !== null) throw new Error(`Vite stopped before readiness: ${startupOutput}`)
    return startupOutput.includes('ready in')
  }, 'Identity management Vite entry must become ready', 30_000)
}

async function fixture(options = {}) {
  const context = await browser.newContext({ viewport: options.mobile ? { width: 390, height: 844 } : { width: 1440, height: 1080 }, reducedMotion: 'reduce' })
  const page = await context.newPage()
  page.setDefaultTimeout(12_000)
  const state = {
    capabilities: { ...fullCapabilities(), ...options.capabilities }, members: clone(options.members ?? initialMembers()),
    requests: [], writes: [], commits: [], errors: [], unexpected: [], roleCount: 1,
    failCapabilities: !!options.failCapabilities, failCounterpart: !!options.failCounterpart,
  }
  page.on('pageerror', error => state.errors.push(error.message))
  await installSession(context, token, proof)
  await context.addInitScript(() => localStorage.setItem('platform-theme', 'light'))
  // Keep the production entry and its StrictMode, router, QueryClient and session gate.
  // Only its App import is replaced by the real UsersPage under test.
  await context.routeWebSocket(() => true, () => {})
  await context.route(url => url.origin === origin && url.pathname === '/src/main.tsx', async route => {
    const response = await route.fetch()
    let body = await response.text()
    const appImport = /import App from ["']\/src\/App\.tsx(?:\?[^"']*)?["']/
    assert.match(body, appImport, 'Mount the actual production UsersPage from Vite')
    body = body.replace(appImport, 'import App from "/src/pages/system/users/UsersPage.tsx"')
    body = body.replace('ReactDOM.createRoot', `import { authStore as identityFixtureAuthStore } from "/src/stores/auth.ts";\nawait identityFixtureAuthStore.getState().initializeSessionCoordination();\nidentityFixtureAuthStore.getState().setProfile(${JSON.stringify(profile)});\nReactDOM.createRoot`)
    await route.fulfill({ response, body })
  })

  const fulfill = async (route, value, status = 200) => {
    try { await route.fulfill({ status, contentType: 'application/json', headers: { 'x-request-id': ids.receipt }, body: JSON.stringify(value) }) }
    catch (error) { if (!/closed|disposed|Invalid Interception|Target|cancelled/i.test(error.message)) throw error }
  }
  const refuse = (route, message, status = 409) => fulfill(route, { code: 'IDENTITY_FIXTURE_REFUSED', message }, status)
  await context.route(url => url.origin === origin && url.pathname.startsWith('/api/'), async route => {
    const request = route.request(), url = new URL(request.url())
    const item = { pathname: url.pathname, method: request.method(), params: Object.fromEntries(url.searchParams), body: request.postData() ? request.postDataJSON() : undefined }
    state.requests.push(item)
    try {
      assert.equal(request.headers().authorization, `Bearer ${token}`, 'All management requests use the shared proven session')
      if (url.pathname === '/api/auth/profile') return fulfill(route, profile)
      if (url.pathname === '/api/auth/permissions') return fulfill(route, { permissions: ['system:user', 'system:data-grant:read'], roles: [] })
      if (url.pathname === '/api/auth/menus') return fulfill(route, [
        { id: id('users-menu'), parentId: null, type: 'MENU', title: '用户管理', path: '/system/users', permission: 'system:user', sortOrder: 1, visible: true, children: [] },
        { id: id('grants-menu'), parentId: null, type: 'MENU', title: '数据授权', path: '/system/data-grants', permission: 'system:data-grant:read', sortOrder: 2, visible: true, children: [] },
      ])
      if (url.pathname === '/api/tenants/options') return fulfill(route, [tenant])
      if (url.pathname === '/api/organizations/tree') {
        assert.equal(item.params.tenantId, tenant.id)
        return fulfill(route, [organization])
      }
      if (url.pathname === '/api/departments/tree') return fulfill(route, [])
      if (url.pathname === '/api/memberships/capabilities') return state.failCapabilities ? refuse(route, '能力依赖暂不可用', 503) : fulfill(route, state.capabilities)
      if (url.pathname === '/api/users' && item.method === 'GET') {
        const keyword = (item.params.keyword ?? '').toLowerCase()
        const records = accounts.filter(account => !keyword || [account.username, account.realName, account.email, account.phone].some(value => value.toLowerCase().includes(keyword)))
          .map(account => ({ ...account, roleCount: account.id === ids['user-a'] ? state.roleCount : 0, memberships: state.members.filter(row => row.userId === account.id).map(row => ({
            organizationId: row.organizationId, organizationName: organization.name, departmentId: row.departmentId, departmentName: null,
            membershipId: row.id, memberType: row.memberType, status: row.status, identityVersion: row.identityVersion,
          })) }))
        return fulfill(route, { records, total: records.length, page: Number(item.params.page ?? 1), size: Number(item.params.size ?? 10) })
      }
      const legacy = /^\/api\/users\/([^/]+)\/memberships$/.exec(url.pathname)
      if (legacy && item.method === 'GET') {
        return fulfill(route, state.members.filter(row => row.userId === legacy[1] && (options.legacyIncludesCustomer || row.memberType === 'EMPLOYEE'))
          .map(row => ({ organizationId: row.organizationId, tenantId: tenant.id, departmentId: row.departmentId, position: row.position,
            memberType: row.memberType, status: row.status, identityVersion: row.identityVersion, membershipId: row.id })))
      }
      if (legacy && item.method === 'PUT') {
        state.writes.push(item)
        assert.deepEqual(Object.keys(item.body), ['items'])
        for (const proposed of item.body.items) {
          assert.deepEqual(Object.keys(proposed).sort(), ['departmentId', 'organizationId', 'position'])
          const employee = state.members.find(row => row.userId === legacy[1] && row.organizationId === proposed.organizationId && row.memberType === 'EMPLOYEE')
          assert(employee, 'This compatibility fixture only updates existing employees')
          employee.departmentId = proposed.departmentId; employee.position = proposed.position
        }
        state.commits.push(item)
        return fulfill(route, { saved: true })
      }
      if (url.pathname === '/api/memberships' && item.method === 'GET') {
        if (state.failCounterpart && !item.params.memberType) return refuse(route, '目标成员权威依赖暂不可用', 503)
        const records = state.members.filter(row => ['userId', 'organizationId', 'memberType', 'status'].every(key => !item.params[key] || row[key] === item.params[key]))
        const page = Number(item.params.page ?? 1), size = Number(item.params.size ?? 10)
        return fulfill(route, { records: records.slice((page - 1) * size, page * size), total: records.length, page, size })
      }
      const match = /^\/api\/memberships\/([^/]+)(?:\/(status|second-identity|replace-identity))?$/.exec(url.pathname)
      if (match) {
        const source = state.members.find(row => row.id === match[1]), operation = match[2]
        if (!source) return refuse(route, '成员不存在', 404)
        if (item.method === 'GET' && !operation) return fulfill(route, source)
        state.writes.push(item)
        if (!state.capabilities.lifecycleEnabled) return refuse(route, '成员生命周期写入尚未开放', 403)
        assert.equal(typeof item.body.reason, 'string')
        assert(item.body.reason.trim(), 'Every lifecycle mutation carries an actual reason')
        if (item.body.expectedIdentityVersion !== source.identityVersion) return refuse(route, '成员身份已变更，请重新读取后重试')
        const forbiddenSelf = source.id === ids['actor-member'] && (!operation || operation === 'status' || operation === 'replace-identity')
        if (forbiddenSelf) return refuse(route, '不能修改当前员工身份', 403)
        if (operation === 'status' && item.method === 'PUT') {
          source.status = item.body.status; source.identityVersion++; source.updatedAt = new Date().toISOString()
          state.commits.push(item)
          return fulfill(route, source)
        }
        if (!operation && item.method === 'DELETE') {
          state.members = state.members.filter(row => row.id !== source.id)
          state.commits.push(item)
          return fulfill(route, { id: source.id, deleted: true, identityVersion: source.identityVersion + 1 })
        }
        if (item.method === 'POST' && (operation === 'second-identity' || operation === 'replace-identity')) {
          if (!state.capabilities.dualMembershipsEnabled || item.body.memberType === 'CUSTOMER' && !state.capabilities.customerMembershipsEnabled) return refuse(route, '身份组合尚未开放', 403)
          assert.notEqual(item.body.memberType, source.memberType)
          if (item.body.memberType === 'CUSTOMER') {
            assert.equal(item.body.accountBindingConfirmed, true)
            assert(item.body.bindingSource?.trim())
          }
          let target = state.members.find(row => row.userId === source.userId && row.organizationId === source.organizationId && row.memberType === item.body.memberType)
          if (target) {
            if (item.body.expectedTargetIdentityVersion !== target.identityVersion) return refuse(route, '目标成员身份已变更')
            if (target.status === 'DISABLED') {
              if (item.body.enableTarget !== true) return refuse(route, '须明确启用目标成员')
              target.status = 'ENABLED'; target.identityVersion++
            }
          } else {
            assert.equal(Object.hasOwn(item.body, 'expectedTargetIdentityVersion'), false)
            target = member(id(`created-${source.userId}-${item.body.memberType}`), source.userId, item.body.memberType, 'ENABLED', 1)
            target.position = ''
            state.members.push(target)
          }
          if (operation === 'replace-identity') { source.status = 'DISABLED'; source.identityVersion++ }
          state.commits.push(item)
          return fulfill(route, { ...target, sourceMembershipId: source.id })
        }
      }
      if (url.pathname === '/api/memberships' && item.method === 'POST') {
        state.writes.push(item)
        if (!state.capabilities.lifecycleEnabled || item.body.memberType === 'CUSTOMER' && !state.capabilities.customerMembershipsEnabled) return refuse(route, '新成员创建尚未开放', 403)
        assert.equal(item.body.userId, ids['user-b'], 'Create binds only the explicitly selected existing account')
        assert.equal(item.body.organizationId, organization.id)
        assert(item.body.reason?.trim())
        assert.equal(state.members.some(row => row.userId === item.body.userId && row.organizationId === item.body.organizationId), false)
        if (item.body.memberType === 'CUSTOMER') { assert.equal(item.body.accountBindingConfirmed, true); assert(item.body.bindingSource?.trim()) }
        const target = member(id(`created-${item.body.userId}-${item.body.memberType}`), item.body.userId, item.body.memberType, 'ENABLED', 1)
        target.position = item.body.position ?? ''
        state.members.push(target); state.commits.push(item)
        return fulfill(route, target)
      }
      state.unexpected.push(item)
      return refuse(route, `Unexpected API ${item.method} ${url.pathname}`, 503)
    } catch (error) {
      state.errors.push(`HTTP fixture: ${error.message}`)
      return refuse(route, 'HTTP fixture assertion failed', 503)
    }
  })
  try {
    await page.goto(`${origin}/system/users`, { waitUntil: 'domcontentloaded' })
    await page.getByRole('tab', { name: '员工成员', exact: true }).waitFor()
    await page.locator(`[data-membership-id="${ids.employee}"]`).waitFor()
    const result = { context, page, state }
    await waitDirectoryReady(result, '员工成员')
    return result
  } catch (error) {
    console.error('Identity management startup diagnostic', { errors: state.errors, unexpected: state.unexpected, body: await page.locator('body').innerText().catch(() => '') })
    await context.close(); throw error
  }
}

async function scenario(name, options, run) {
  const f = await fixture(options)
  try {
    await run(f)
    assert.deepEqual(f.state.errors, [], 'Production components and HTTP fixture assertions must not fail')
    assert.deepEqual(f.state.unexpected, [], 'All HTTP dependencies are explicitly isolated')
    scenarios.push({ name, status: 'PASS', writes: f.state.writes.length, commits: f.state.commits.length })
    console.log(`PASS identity management: ${name}`)
  } catch (error) {
    console.error('Identity management scenario diagnostic', { name, errors: f.state.errors, unexpected: f.state.unexpected,
      writes: f.state.writes, body: await f.page.locator('body').innerText().catch(() => '') })
    throw error
  } finally { await f.context.close() }
}

const memberRow = (f, memberId) => f.page.locator(`[data-membership-id="${memberId}"]`)
const accountRow = (f, username) => f.page.getByRole('row').filter({ has: f.page.getByText(`@${username}`, { exact: true }) })
async function waitDirectoryReady(f, label) {
  if (f.state.failCapabilities) {
    await f.page.getByText('无法读取成员写入能力，所有生命周期操作暂不可用', { exact: true }).waitFor()
    return
  }
  const gates = f.state.capabilities
  const expected = !gates.lifecycleEnabled ? '服务端尚未开放成员生命周期写入'
    : label === '客户成员' && !gates.customerMembershipsEnabled ? '服务端尚未开放客户身份创建'
      : '成员创建与生命周期写入已由服务器授权开放。'
  await f.page.getByText(expected, { exact: true }).waitFor()
}
async function view(f, label) {
  await f.page.getByRole('tab', { name: label, exact: true }).click()
  if (label === '全局账号') await f.page.getByRole('heading', { name: '全局账号', exact: true }).waitFor()
  else {
    await f.page.getByRole('heading', { name: new RegExp(`^${label}`) }).waitFor()
    await waitDirectoryReady(f, label)
  }
}
async function open(f, memberId, button, title) {
  await memberRow(f, memberId).getByRole('button', { name: button, exact: true }).click()
  const dialog = f.page.getByRole('dialog')
  await dialog.getByRole('heading', { name: title, exact: true }).waitFor()
  await waitState(async () => !await dialog.getByRole('button', { name: title, exact: true }).isDisabled(), 'Read exact source and counterpart proof before enabling the action')
  return dialog
}
async function submit(f, dialog, label, expectedCommits = f.state.commits.length + 1) {
  await dialog.getByRole('button', { name: label, exact: true }).click()
  await waitState(() => f.state.commits.length === expectedCommits, `${label} must commit through the exact lifecycle endpoint`)
  await dialog.waitFor({ state: 'hidden' })
}
async function screenshot(f, filename) {
  await f.page.screenshot({ path: path.join(reportDirectory, filename), fullPage: true, animations: 'disabled' })
  screenshots.push(filename)
}
async function noDocumentOverflow(f) {
  assert(await f.page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), 'Horizontal table scrolling must remain inside its container')
}
async function bindCreateAccount(f, dialog) {
  await dialog.getByRole('textbox', { name: '检索既有账号', exact: true }).fill('shared-contact')
  const candidates = dialog.getByRole('button').filter({ hasText: 'same-contact-' })
  await waitState(async () => await candidates.count() === 2, 'Matching contact details must remain two explicit account choices')
  assert.equal(await dialog.getByTestId('selected-account-id').count(), 0, 'A contact match never auto-selects an account')
  await candidates.filter({ hasText: ids['user-b'] }).click()
  assert.equal(await dialog.getByTestId('selected-account-id').innerText(), ids['user-b'])
  await dialog.getByRole('combobox', { name: '成员目标组织', exact: true }).click()
  await f.page.getByRole('button').filter({ hasText: `${tenant.name} / ${organization.name}` }).click()
  await waitState(async () => !await dialog.getByRole('button', { name: '新增客户关系', exact: true }).isDisabled(), 'Known empty counterpart discovery enables creation intent validation')
}

try {
  await fs.mkdir(reportDirectory, { recursive: true })
  await startServer()
  browser = await launchBrowser()

  await scenario('desktop three views and customer employee-field isolation', {}, async f => {
    assert.equal(await f.page.getByRole('tab').count(), 3)
    assert.equal(await memberRow(f, ids.employee).getAttribute('data-member-type'), 'EMPLOYEE')
    await memberRow(f, ids.employee).getByRole('link', { name: '员工授权', exact: true }).waitFor()
    const grantUrl = new URL(await memberRow(f, ids.employee).getByRole('link', { name: '员工授权', exact: true }).getAttribute('href'), origin)
    assert.equal(grantUrl.searchParams.get('membershipId'), ids.employee)
    assert.equal(grantUrl.searchParams.get('userId'), ids['user-a'])
    await screenshot(f, 'desktop-employees.png')
    await view(f, '客户成员')
    await memberRow(f, ids.customer).waitFor()
    assert.equal(await f.page.getByRole('columnheader', { name: '岗位', exact: true }).count(), 0)
    assert.equal(await memberRow(f, ids.customer).getByRole('link', { name: /员工授权/ }).count(), 0)
    assert.equal(await memberRow(f, ids.customer).getByRole('button', { name: /角色|岗位/ }).count(), 0)
    assert.match(await memberRow(f, ids.customer).innerText(), /客户[\s\S]*已停用[\s\S]*v12/)
    await screenshot(f, 'desktop-customers.png')
    await view(f, '全局账号')
    await f.page.getByText('@same-contact-a', { exact: true }).waitFor()
    await f.page.getByText('@same-contact-b', { exact: true }).waitFor()
    assert.match(await f.page.locator('body').innerText(), /账号启停、密码和删除影响此账号的所有成员身份/)
    assert.equal(f.state.writes.length, 0)
  })

  await scenario('mobile member directory and dialog fit viewport', { mobile: true }, async f => {
    await noDocumentOverflow(f)
    await screenshot(f, 'mobile-employees.png')
    await view(f, '客户成员')
    await memberRow(f, ids.customer).waitFor()
    await noDocumentOverflow(f)
    await screenshot(f, 'mobile-customers.png')
    const dialog = await open(f, ids.customer, '启用', '启用成员')
    await noDocumentOverflow(f)
    const bounds = await dialog.boundingBox()
    assert(bounds.x >= 0 && bounds.x + bounds.width <= 391)
    await screenshot(f, 'mobile-member-action.png')
  })

  await scenario('lifecycle switch closed keeps all typed actions read only', { capabilities: { lifecycleEnabled: false } }, async f => {
    for (const label of ['停用', '第二身份', '替换', '删除关系']) assert(await memberRow(f, ids.employee).getByRole('button', { name: label, exact: true }).isDisabled())
    assert(await f.page.getByRole('button', { name: '新增员工关系', exact: true }).isDisabled())
    await view(f, '客户成员')
    assert(await f.page.getByRole('button', { name: '新增客户关系', exact: true }).isDisabled())
    assert(await memberRow(f, ids.customer).getByRole('button', { name: '启用', exact: true }).isDisabled())
    assert.equal(f.state.writes.length, 0)
  })

  await scenario('customer switch closes customer targets while retaining customer status administration', { capabilities: { customerMembershipsEnabled: false } }, async f => {
    assert.equal(await f.page.getByRole('button', { name: '新增员工关系', exact: true }).isDisabled(), false)
    for (const label of ['第二身份', '替换']) assert(await memberRow(f, ids.employee).getByRole('button', { name: label, exact: true }).isDisabled())
    await view(f, '客户成员')
    assert(await f.page.getByRole('button', { name: '新增客户关系', exact: true }).isDisabled())
    assert.equal(await memberRow(f, ids.customer).getByRole('button', { name: '启用', exact: true }).isDisabled(), false)
    assert.equal(await memberRow(f, ids.customer).getByRole('button', { name: '第二身份', exact: true }).isDisabled(), false)
    assert.equal(f.state.writes.length, 0)
  })

  await scenario('dual switch closes second and replace for both source types', { capabilities: { dualMembershipsEnabled: false } }, async f => {
    for (const source of [ids.employee, ids.customer]) {
      if (source === ids.customer) await view(f, '客户成员')
      for (const label of ['第二身份', '替换']) assert(await memberRow(f, source).getByRole('button', { name: label, exact: true }).isDisabled())
    }
    assert.equal(f.state.writes.length, 0)
  })

  await scenario('unavailable capability authority never enables lifecycle writes', { failCapabilities: true }, async f => {
    await f.page.getByText('无法读取成员写入能力，所有生命周期操作暂不可用', { exact: true }).waitFor()
    assert(await f.page.getByRole('button', { name: '新增员工关系', exact: true }).isDisabled())
    assert(await memberRow(f, ids.employee).getByRole('button', { name: '停用', exact: true }).isDisabled())
    assert.equal(f.state.writes.length, 0)
  })

  await scenario('exact current employee self protection does not protect same-account same-org customer', {
    members: [...initialMembers(), member(id('actor-customer'), ids.actor, 'CUSTOMER', 'DISABLED', 5)],
  }, async f => {
    const current = memberRow(f, ids['actor-member'])
    for (const label of ['停用', '替换', '删除关系']) assert(await current.getByRole('button', { name: label, exact: true }).isDisabled())
    assert.equal(await current.getByRole('button', { name: '第二身份', exact: true }).isDisabled(), false)
    await view(f, '客户成员')
    for (const label of ['启用', '替换', '删除关系']) assert.equal(await memberRow(f, id('actor-customer')).getByRole('button', { name: label, exact: true }).isDisabled(), false)
    assert.equal(f.state.writes.length, 0)
  })

  await scenario('independent employee disable customer enable and customer delete', {}, async f => {
    const customerBefore = clone(f.state.members.find(row => row.id === ids.customer))
    let dialog = await open(f, ids.employee, '停用', '停用成员')
    await dialog.getByRole('textbox', { name: /^本次操作原因/ }).fill('员工离职，保留其客户关系')
    await submit(f, dialog, '停用成员')
    assert.equal(f.state.members.find(row => row.id === ids.employee).status, 'DISABLED')
    assert.deepEqual(f.state.members.find(row => row.id === ids.customer), customerBefore)
    assert.equal(f.state.roleCount, 1, 'Temporary employee disable keeps stored role configuration')
    const employeeAfter = clone(f.state.members.find(row => row.id === ids.employee))
    await view(f, '客户成员')
    dialog = await open(f, ids.customer, '启用', '启用成员')
    await dialog.getByRole('textbox', { name: /^本次操作原因/ }).fill('恢复客户服务资格')
    await submit(f, dialog, '启用成员')
    assert.deepEqual(f.state.members.find(row => row.id === ids.employee), employeeAfter)
    await memberRow(f, ids.customer).getByRole('button', { name: '停用', exact: true }).waitFor()
    dialog = await open(f, ids.customer, '删除关系', '删除成员关系')
    assert.match(await dialog.innerText(), /全局账号及其他成员关系不会删除/)
    await dialog.getByRole('textbox', { name: /^本次操作原因/ }).fill('解除误绑定客户关系')
    await submit(f, dialog, '删除成员关系')
    assert.deepEqual(f.state.members.find(row => row.id === ids.employee), employeeAfter)
    assert.equal(f.state.members.some(row => row.id === ids.customer), false)
    assert.deepEqual(f.state.writes.map(item => [item.method, item.pathname]), [
      ['PUT', `/api/memberships/${ids.employee}/status`], ['PUT', `/api/memberships/${ids.customer}/status`], ['DELETE', `/api/memberships/${ids.customer}`],
    ])
  })

  await scenario('second identity explicitly reuses disabled target; replace disables source', {}, async f => {
    let dialog = await open(f, ids.employee, '第二身份', '新增第二身份')
    assert.match(await dialog.innerText(), /源成员 · 保留原状态/)
    assert.match(await dialog.innerText(), /目标成员 · 显式复用/)
    await dialog.getByRole('textbox', { name: /^本次操作原因/ }).fill('员工同时办理客户业务')
    await dialog.getByRole('button', { name: '新增第二身份', exact: true }).click()
    await dialog.getByRole('alert').filter({ hasText: /明确确认复用/ }).waitFor()
    assert.equal(f.state.writes.length, 0)
    await dialog.getByRole('checkbox', { name: /我已核对目标成员 ID/ }).check()
    await dialog.getByRole('button', { name: '新增第二身份', exact: true }).click()
    await dialog.getByRole('alert').filter({ hasText: /明确确认启用/ }).waitFor()
    assert.equal(f.state.writes.length, 0)
    await dialog.getByRole('checkbox', { name: /明确启用已停用的目标/ }).check()
    await dialog.getByRole('textbox', { name: /^绑定依据/ }).fill('管理员核实原账号身份与客户合同')
    await dialog.getByRole('button', { name: '新增第二身份', exact: true }).click()
    await dialog.getByRole('alert').filter({ hasText: /明确确认账号归属/ }).waitFor()
    assert.equal(f.state.writes.length, 0)
    await dialog.getByRole('checkbox', { name: /已核实该既有账号的归属/ }).check()
    await screenshot(f, 'desktop-second-identity-confirmation.png')
    await submit(f, dialog, '新增第二身份')
    const source = f.state.members.find(row => row.id === ids.employee), target = f.state.members.find(row => row.id === ids.customer)
    assert.equal(source.status, 'ENABLED'); assert.equal(source.identityVersion, 7)
    assert.equal(target.status, 'ENABLED'); assert.equal(target.identityVersion, 13)
    assert.deepEqual(f.state.writes[0].body, { expectedIdentityVersion: 7, memberType: 'CUSTOMER', expectedTargetIdentityVersion: 12, enableTarget: true,
      reason: '员工同时办理客户业务', accountBindingConfirmed: true, bindingSource: '管理员核实原账号身份与客户合同' })
    dialog = await open(f, ids.employee, '替换', '替换身份')
    assert.match(await dialog.innerText(), /源成员 · 完成后停用/)
    assert.match(await dialog.innerText(), /历史通知不会改绑目标/)
    await dialog.getByRole('checkbox', { name: /我已核对目标成员 ID/ }).check()
    await dialog.getByRole('checkbox', { name: /已核实该既有账号的归属/ }).check()
    await dialog.getByRole('textbox', { name: /^绑定依据/ }).fill('原账号与客户档案人工核验一致')
    await dialog.getByRole('textbox', { name: /^本次操作原因/ }).fill('员工退出后仅保留客户身份')
    await submit(f, dialog, '替换身份')
    assert.equal(source.status, 'DISABLED'); assert.equal(source.identityVersion, 8)
    assert.equal(target.identityVersion, 13)
    assert.equal(f.state.writes[1].pathname, `/api/memberships/${ids.employee}/replace-identity`)
    assert.equal(f.state.writes[1].body.expectedTargetIdentityVersion, 13)
    assert.equal(Object.hasOwn(f.state.writes[1].body, 'enableTarget'), false)
  })

  await scenario('stale source version is refused without automatic mutation retry or optimistic success', {}, async f => {
    let dialog = await open(f, ids.employee, '停用', '停用成员')
    f.state.members.find(row => row.id === ids.employee).identityVersion = 8
    await dialog.getByRole('textbox', { name: /^本次操作原因/ }).fill('并发修改后的员工离职')
    await dialog.getByRole('button', { name: '停用成员', exact: true }).click()
    await dialog.getByRole('alert').filter({ hasText: /成员身份已变更/ }).waitFor()
    assert.equal(f.state.writes.length, 1)
    assert.equal(f.state.writes[0].body.expectedIdentityVersion, 7)
    assert.equal(f.state.commits.length, 0)
    assert.equal(f.state.members.find(row => row.id === ids.employee).status, 'ENABLED')
    assert.match(await dialog.getByRole('alert').innerText(), new RegExp(ids.receipt))
    await dialog.getByRole('button', { name: '取消', exact: true }).click()
    dialog = await open(f, ids.employee, '停用', '停用成员')
    await dialog.getByRole('textbox', { name: /^本次操作原因/ }).fill('重新核验 v8 后确认员工离职')
    await submit(f, dialog, '停用成员')
    assert.equal(f.state.writes.length, 2)
    assert.equal(f.state.writes[1].body.expectedIdentityVersion, 8)
  })

  await scenario('customer creation selects an exact existing account and requires binding evidence', {}, async f => {
    await view(f, '客户成员')
    await f.page.getByRole('button', { name: '新增客户关系', exact: true }).click()
    const dialog = f.page.getByRole('dialog')
    await dialog.getByRole('heading', { name: '新增客户关系', exact: true }).waitFor()
    assert.equal(await dialog.getByRole('textbox', { name: /岗位/ }).count(), 0)
    await bindCreateAccount(f, dialog)
    await dialog.getByRole('textbox', { name: /^本次操作原因/ }).fill('为核实后的既有账号开通客户服务')
    await dialog.getByRole('textbox', { name: /^绑定依据/ }).fill('人工核验客户资料与账号完整 ID')
    await dialog.getByRole('button', { name: '新增客户关系', exact: true }).click()
    await dialog.getByRole('alert').filter({ hasText: /明确确认账号归属/ }).waitFor()
    assert.equal(f.state.writes.length, 0)
    await dialog.getByRole('checkbox', { name: /已核实该既有账号的归属/ }).check()
    await screenshot(f, 'desktop-customer-account-binding.png')
    await submit(f, dialog, '新增客户关系')
    assert.deepEqual(f.state.writes[0].body, { userId: ids['user-b'], organizationId: organization.id, memberType: 'CUSTOMER',
      reason: '为核实后的既有账号开通客户服务', accountBindingConfirmed: true, bindingSource: '人工核验客户资料与账号完整 ID' })
    assert.equal(f.state.requests.some(item => item.pathname === '/api/users' && item.method !== 'GET'), false, 'No implicit global account creation/merge')
  })

  await scenario('unknown target dependency blocks combination instead of assuming absence', { failCounterpart: true }, async f => {
    await memberRow(f, ids.employee).getByRole('button', { name: '第二身份', exact: true }).click()
    const dialog = f.page.getByRole('dialog')
    await dialog.getByText('目标成员读取失败，不能推断目标不存在', { exact: true }).waitFor()
    assert(await dialog.getByRole('button', { name: '新增第二身份', exact: true }).isDisabled())
    assert.equal(f.state.writes.length, 0)
  })

  await scenario('legacy employee batch edits retain disabled state and exclude customer fields', {
    members: initialMembers().map(row => row.id === ids.employee ? { ...row, status: 'DISABLED' } : row),
  }, async f => {
    const customerBefore = clone(f.state.members.find(row => row.id === ids.customer))
    await view(f, '全局账号')
    const targetAccount = accountRow(f, 'same-contact-a')
    await targetAccount.getByRole('button', { name: '员工成员批量维护', exact: true }).waitFor()
    assert.equal(await targetAccount.getByRole('button', { name: '分配角色', exact: true }).count(), 0, 'Disabled employees cannot provide a role assignment target')
    await targetAccount.getByRole('button', { name: '员工成员批量维护', exact: true }).click()
    const dialog = f.page.getByRole('dialog')
    await dialog.getByRole('heading', { name: '员工成员关系 · 同名账号甲', exact: true }).waitFor()
    await waitState(async () => !await dialog.getByRole('button', { name: '保存成员关系', exact: true }).isDisabled(), 'Legacy bulk must read explicitly typed employee rows')
    assert.equal(await dialog.getByPlaceholder('职位,如:数据分析师').count(), 1)
    await dialog.getByPlaceholder('职位,如:数据分析师').fill('已停用员工资料更新')
    await submit(f, dialog, '保存成员关系')
    assert.deepEqual(f.state.writes[0].body, { items: [{ organizationId: organization.id, departmentId: null, position: '已停用员工资料更新' }] })
    assert.equal(f.state.members.find(row => row.id === ids.employee).status, 'DISABLED')
    assert.equal(f.state.members.find(row => row.id === ids.employee).identityVersion, 7)
    assert.deepEqual(f.state.members.find(row => row.id === ids.customer), customerBefore)
  })

  await scenario('legacy bulk refuses an unexpected customer row instead of making it editable', { legacyIncludesCustomer: true }, async f => {
    await view(f, '全局账号')
    const targetAccount = accountRow(f, 'same-contact-a')
    await targetAccount.getByRole('button', { name: '员工成员批量维护', exact: true }).click()
    const dialog = f.page.getByRole('dialog')
    await dialog.getByText('员工批量关系响应包含未确认的身份，请重新读取', { exact: true }).waitFor()
    assert(await dialog.getByRole('button', { name: '保存成员关系', exact: true }).isDisabled())
    assert.equal(await dialog.getByPlaceholder('职位,如:数据分析师').count(), 0)
    assert.equal(f.state.writes.length, 0)
  })

  await fs.writeFile(path.join(reportDirectory, 'identity-management-browser.json'), JSON.stringify({
    status: 'PASS', scope: 'Production React/Vite UsersPage with isolated HTTP fixtures; no database assertions',
    browser: await browser.version(), scenarios, screenshots,
  }, null, 2) + '\n')
  console.log(`Identity management browser passed (${scenarios.length} scenarios). Report: ${reportDirectory}`)
} finally {
  await browser?.close()
  if (vite && vite.exitCode === null) {
    const exited = new Promise(resolve => vite.once('exit', resolve))
    vite.kill()
    await Promise.race([exited, pause(5000)])
    if (vite.exitCode === null) vite.kill('SIGKILL')
  }
}
