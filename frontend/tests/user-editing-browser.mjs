/** Real production React pages with isolated HTTP persistence. This does not
 * establish database, identity service, or deployed environment correctness. */
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
const reportPath = process.env.USER_EDITING_BROWSER_REPORT_DIR
assert(reportPath && path.isAbsolute(reportPath), 'Set an absolute USER_EDITING_BROWSER_REPORT_DIR')
const reportDirectory = path.resolve(reportPath)
for (const directory of ['src', 'tests', 'node_modules']) {
  const relative = path.relative(path.join(workspace, directory), reportDirectory)
  assert(relative.startsWith('..') || path.isAbsolute(relative), 'Browser reports must not overwrite source or dependencies')
}
const host = process.platform === 'win32' ? '::1' : '127.0.0.1'
const id = label => fixtureUuid(`user-editing:${label}`)
const ids = Object.fromEntries(['actor', 'account', 'tenant', 'organization', 'department', 'employee', 'customer', 'actor-member', 'role', 'system', 'receipt'].map(label => [label, id(label)]))
const token = 'user-editing-browser-session'
const tenant = { id: ids.tenant, name: '资料编辑租户', code: 'editing', isPlatform: true, status: 'ENABLED' }
const organization = { id: ids.organization, tenantId: tenant.id, parentId: null, name: '资料编辑组织', code: 'editing-org', status: 'ENABLED', children: [] }
const department = { id: ids.department, organizationId: organization.id, name: '售后服务部', code: 'service', status: 'ENABLED', children: [] }
const account = { id: ids.account, username: 'editable-user', realName: '待编辑员工', email: 'old@example.test', phone: '13800138000', gender: 'UNKNOWN',
  status: 'ENABLED', createdAt: '2026-10-24T00:00:00Z', lastLoginAt: null, roleCount: 0 }
const member = (membershipId, memberType) => ({ membershipId, organizationId: organization.id, organizationName: organization.name,
  tenantId: tenant.id, departmentId: null, departmentName: null, position: memberType === 'EMPLOYEE' ? '原岗位' : '', memberType, status: 'ENABLED', identityVersion: 2 })
const profile = profileWithIdentity({
  user: { ...account, id: ids.actor, username: 'fixture-admin', realName: '验收管理员', avatar: '', isSuperAdmin: false }, tenant,
  organization: { id: organization.id, name: organization.name, path: `/${organization.id}` }, platformContext: true,
  applications: [{ id: ids.system, name: '系统管理', code: 'system', icon: '', basePath: '/system' }],
  memberships: [{ userId: ids.actor, tenantId: tenant.id, tenantName: tenant.name, organizationId: organization.id, organizationName: organization.name,
    organizationPath: `/${organization.id}`, membershipId: ids['actor-member'], memberType: 'EMPLOYEE', identityVersion: '3', status: 'ENABLED', consoleSelectable: true, selectionDisabledReason: null }],
}, { membershipId: ids['actor-member'], identityVersion: '3', accountVersion: '0' })
const proof = identityForProfile(profile)
let browser, vite, origin, startupOutput = ''
const scenarios = []

async function startServer() {
  const port = await new Promise((resolve, reject) => {
    const server = net.createServer().on('error', reject)
    server.listen(0, host, () => { const port = server.address().port; server.close(error => error ? reject(error) : resolve(port)) })
  })
  origin = `http://${host.includes(':') ? `[${host}]` : host}:${port}`
  vite = spawn(process.execPath, [path.join(workspace, 'node_modules/vite/bin/vite.js'), '--host', host, '--port', String(port), '--strictPort'], {
    cwd: workspace, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'],
  })
  let spawnError
  vite.on('error', error => { spawnError = error })
  for (const stream of [vite.stdout, vite.stderr]) stream.on('data', chunk => { startupOutput = (startupOutput + chunk).slice(-6000) })
  await waitState(() => {
    if (spawnError) throw spawnError
    if (vite.exitCode !== null) throw new Error(`Vite stopped before readiness: ${startupOutput}`)
    return startupOutput.includes('ready in')
  }, 'User editing Vite entry becomes ready', 30_000)
}

async function fixture(options = {}) {
  const context = await browser.newContext({ viewport: options.mobile ? { width: 390, height: 844 } : { width: 1440, height: 1000 }, reducedMotion: 'reduce' })
  const page = await context.newPage()
  page.setDefaultTimeout(12_000)
  const state = { account: structuredClone(account), members: [member(ids.employee, 'EMPLOYEE'), member(ids.customer, 'CUSTOMER')],
    roles: [], requests: [], writes: [], commits: [], errors: [], unexpected: [], failNextProfileSave: !!options.failNextProfileSave }
  page.on('pageerror', error => state.errors.push(error.message))
  await installSession(context, token, proof)
  await context.addInitScript(() => localStorage.setItem('platform-theme', 'light'))
  await context.routeWebSocket(() => true, () => {})
  await context.route(url => url.origin === origin && url.pathname === '/src/main.tsx', async route => {
    const response = await route.fetch()
    const source = options.customer ? '/src/pages/system/customers/CustomersPage.tsx' : '/src/pages/system/users/UsersPage.tsx'
    const importPattern = /import App from ["']\/src\/App\.tsx(?:\?[^"']*)?["']/
    let body = await response.text()
    assert.match(body, importPattern, 'Mount the production directory from the production Vite entry')
    body = body.replace(importPattern, `import App from "${source}"`)
    body = body.replace('ReactDOM.createRoot', `import { authStore as editingFixtureAuthStore } from "/src/stores/auth.ts";\nawait editingFixtureAuthStore.getState().initializeSessionCoordination();\neditingFixtureAuthStore.getState().setProfile(${JSON.stringify(profile)});\nReactDOM.createRoot`)
    await route.fulfill({ response, body })
  })
  const fulfill = async (route, value, status = 200) => {
    try { await route.fulfill({ status, contentType: 'application/json', headers: { 'x-request-id': ids.receipt }, body: JSON.stringify(value) }) }
    catch (error) { if (!/closed|disposed|Invalid Interception|Target|cancelled/i.test(error.message)) throw error }
  }
  await context.route(url => url.origin === origin && url.pathname.startsWith('/api/'), async route => {
    const request = route.request(), url = new URL(request.url())
    const item = { pathname: url.pathname, method: request.method(), params: Object.fromEntries(url.searchParams), body: request.postData() ? request.postDataJSON() : undefined }
    state.requests.push(item)
    try {
      assert.equal(request.headers().authorization, `Bearer ${token}`)
      if (url.pathname === '/api/auth/profile') return fulfill(route, profile)
      if (url.pathname === '/api/auth/permissions') return fulfill(route, { permissions: ['system:user', 'system:customer', 'system:data-grant:read'], roles: [] })
      if (url.pathname === '/api/auth/menus') return fulfill(route, [
        { id: id('users-menu'), parentId: null, type: 'MENU', title: '用户管理', path: '/system/users', permission: 'system:user', sortOrder: 1, visible: true, children: [] },
        { id: id('grants-menu'), parentId: null, type: 'MENU', title: '数据授权', path: '/system/data-grants', permission: 'system:data-grant:read', sortOrder: 2, visible: true, children: [] },
      ])
      if (url.pathname === '/api/tenants/options') return fulfill(route, [tenant])
      if (url.pathname === '/api/organizations/tree') return fulfill(route, [organization])
      if (url.pathname === '/api/departments/tree') return fulfill(route, [department])
      if (url.pathname === '/api/memberships/capabilities') return fulfill(route, { lifecycleEnabled: true, customerMembershipsEnabled: true, dualMembershipsEnabled: true })
      if (url.pathname === '/api/users' && item.method === 'GET') {
        const memberships = state.members.filter(row => !item.params.memberType || row.memberType === item.params.memberType)
        return fulfill(route, { records: [{ ...state.account, memberships, roleCount: state.roles.length }], total: 1, page: 1, size: 10 })
      }
      if (url.pathname === `/api/users/${ids.account}` && item.method === 'PUT') {
        state.writes.push(item)
        assert.deepEqual(Object.keys(item.body).sort(), ['email', 'gender', 'phone', 'realName'])
        if (state.failNextProfileSave) {
          state.failNextProfileSave = false
          return fulfill(route, { code: 'SAVE_REFUSED', message: '资料服务暂不可用，请重试' }, 503)
        }
        Object.assign(state.account, item.body); state.commits.push(item)
        return fulfill(route, { saved: true })
      }
      if (url.pathname === `/api/users/${ids.account}/memberships`) {
        if (item.method === 'GET') return fulfill(route, state.members.filter(row => row.memberType === 'EMPLOYEE'))
        if (item.method === 'PUT') {
          state.writes.push(item)
          assert.deepEqual(Object.keys(item.body), ['items'])
          assert.equal(item.body.items.length, 1, 'Editing an employee must preserve the separate customer relation')
          for (const proposed of item.body.items) {
            assert.deepEqual(Object.keys(proposed).sort(), ['departmentId', 'organizationId', 'position'])
            assert.equal(proposed.organizationId, organization.id)
            Object.assign(state.members.find(row => row.memberType === 'EMPLOYEE'), proposed)
          }
          state.commits.push(item)
          return fulfill(route, { saved: true })
        }
      }
      if (url.pathname === `/api/users/${ids.account}/roles`) {
        if (item.method === 'GET') return fulfill(route, state.roles)
        if (item.method === 'PUT') { state.writes.push(item); state.roles = item.body.items; state.commits.push(item); return fulfill(route, { saved: true }) }
      }
      if (url.pathname === '/api/roles' && item.method === 'GET') return fulfill(route, { records: [{ id: ids.role, organizationId: organization.id, name: '售后主管', code: 'service-lead', menuCount: 1, builtIn: false }], total: 1, page: 1, size: 200 })
      if (url.pathname === '/api/memberships' && item.method === 'GET') return fulfill(route, { records: state.members.map(row => ({ ...row, id: row.membershipId, userId: ids.account })), total: 2, page: 1, size: 20 })
      state.unexpected.push(item)
      return fulfill(route, { code: 'UNEXPECTED_API', message: `Unexpected ${item.method} ${item.pathname}` }, 503)
    } catch (error) {
      state.errors.push(`HTTP fixture: ${error.message}`)
      return fulfill(route, { code: 'FIXTURE_FAILED', message: error.message }, 503)
    }
  })
  try {
    await page.goto(`${origin}/system/${options.customer ? 'customers' : 'users'}`, { waitUntil: 'domcontentloaded' })
    await page.getByRole('heading', { name: options.customer ? '客户管理' : '用户管理', exact: true }).waitFor()
    await page.getByRole('row').filter({ has: page.getByText('@editable-user', { exact: true }) }).waitFor()
    return { context, page, state }
  } catch (error) { await context.close(); throw error }
}

const accountRow = f => f.page.getByRole('row').filter({ has: f.page.getByText('@editable-user', { exact: true }) })
async function openEdit(f) {
  await accountRow(f).getByRole('button', { name: '编辑', exact: true }).click()
  const dialog = f.page.getByRole('dialog')
  await dialog.getByRole('heading', { name: /^编辑用户/ }).waitFor()
  return dialog
}
async function scenario(name, options, run) {
  const f = await fixture(options)
  try {
    await run(f)
    assert.deepEqual(f.state.errors, [], 'Production components and HTTP assertions remain valid')
    assert.deepEqual(f.state.unexpected, [], 'All HTTP dependencies are isolated')
    scenarios.push({ name, status: 'PASS', writes: f.state.writes.length, commits: f.state.commits.length })
    console.log(`PASS user editing: ${name}`)
  } catch (error) {
    await f.page.screenshot({ path: path.join(reportDirectory, `failure-${scenarios.length + 1}.png`), fullPage: true }).catch(() => {})
    console.error('User editing scenario diagnostic', { name, errors: f.state.errors, unexpected: f.state.unexpected,
      writes: f.state.writes, body: await f.page.locator('body').innerText().catch(() => '') })
    throw error
  } finally { await f.context.close() }
}
async function save(f, dialog, label) {
  const count = f.state.commits.length + 1
  await dialog.getByRole('button', { name: label, exact: true }).click()
  await waitState(() => f.state.commits.length === count, `${label} commits exactly once`)
  await dialog.waitFor({ state: 'hidden' })
}

try {
  await fs.mkdir(reportDirectory, { recursive: true })
  await startServer()
  browser = await launchBrowser()
  await scenario('employee profile edits save and refresh the production table', {}, async f => {
    const dialog = await openEdit(f)
    assert(await dialog.getByRole('textbox', { name: /^用户名/ }).isDisabled())
    assert.equal(await dialog.getByRole('textbox', { name: /^初始密码/ }).count(), 0)
    await dialog.getByRole('textbox', { name: /^姓名/ }).fill('更新后的员工')
    await dialog.getByRole('textbox', { name: '邮箱', exact: true }).fill('edited@example.test')
    await dialog.getByRole('textbox', { name: '电话', exact: true }).fill('13900139000')
    await dialog.getByRole('combobox', { name: '性别', exact: true }).click()
    await f.page.getByRole('option', { name: '女', exact: true }).click()
    await save(f, dialog, '保存更改')
    assert.deepEqual(f.state.writes[0].body, { realName: '更新后的员工', email: 'edited@example.test', phone: '13900139000', gender: 'FEMALE' })
    await accountRow(f).getByRole('cell', { name: '更新后的员工', exact: true }).waitFor()
    const reopened = await openEdit(f)
    assert.equal(await reopened.getByRole('textbox', { name: '邮箱', exact: true }).inputValue(), 'edited@example.test')
    await f.page.screenshot({ path: path.join(reportDirectory, 'employee-profile-desktop.png'), fullPage: true })
  })
  await scenario('a failed profile update keeps the draft and retries the requested fields', { failNextProfileSave: true }, async f => {
    const dialog = await openEdit(f)
    await dialog.getByRole('textbox', { name: /^姓名/ }).fill('失败后保留的草稿')
    await dialog.getByRole('button', { name: '保存更改', exact: true }).click()
    await f.page.getByText(/^资料服务暂不可用，请重试/).waitFor()
    assert.equal(await dialog.getByRole('textbox', { name: /^姓名/ }).inputValue(), '失败后保留的草稿')
    assert.equal(f.state.commits.length, 0)
    assert.equal(f.state.writes.length, 1)
    await save(f, dialog, '保存更改')
    assert.equal(f.state.writes.length, 2)
    assert.deepEqual(f.state.writes[0].body, f.state.writes[1].body)
  })
  await scenario('clearing optional contact fields sends explicit empty strings', {}, async f => {
    const dialog = await openEdit(f)
    await dialog.getByRole('textbox', { name: '邮箱', exact: true }).fill('')
    await dialog.getByRole('textbox', { name: '电话', exact: true }).fill('')
    await save(f, dialog, '保存更改')
    assert.equal(f.state.writes[0].body.email, '')
    assert.equal(f.state.writes[0].body.phone, '')
    const reopened = await openEdit(f)
    assert.equal(await reopened.getByRole('textbox', { name: '邮箱', exact: true }).inputValue(), '')
    assert.equal(await reopened.getByRole('textbox', { name: '电话', exact: true }).inputValue(), '')
  })
  await scenario('employee department and position save without touching its customer relation', {}, async f => {
    const customerBefore = structuredClone(f.state.members.find(row => row.memberType === 'CUSTOMER'))
    await accountRow(f).getByRole('button', { name: '员工成员批量维护', exact: true }).click()
    const dialog = f.page.getByRole('dialog')
    await dialog.getByRole('heading', { name: /^员工成员关系/ }).waitFor()
    await dialog.getByPlaceholder('职位,如:数据分析师').fill('售后主管')
    await dialog.getByRole('combobox', { name: '选择部门(可空)', exact: true }).click()
    await f.page.getByRole('button', { name: /售后服务部/ }).click()
    await save(f, dialog, '保存成员关系')
    assert.deepEqual(f.state.writes[0].body, { items: [{ organizationId: ids.organization, departmentId: ids.department, position: '售后主管' }] })
    assert.deepEqual(f.state.members.find(row => row.memberType === 'CUSTOMER'), customerBefore)
    await accountRow(f).getByRole('button', { name: '员工成员批量维护', exact: true }).click()
    assert.equal(await f.page.getByPlaceholder('职位,如:数据分析师').inputValue(), '售后主管')
  })
  await scenario('employee role assignment and detail-to-edit entry are restored', {}, async f => {
    await accountRow(f).getByRole('button', { name: '分配角色', exact: true }).click()
    let dialog = f.page.getByRole('dialog')
    await dialog.getByRole('checkbox', { name: '角色 售后主管', exact: true }).check()
    await save(f, dialog, '保存角色分配')
    assert.deepEqual(f.state.roles, [{ roleId: ids.role, organizationId: ids.organization }])
    await accountRow(f).getByRole('button', { name: '分配角色', exact: true }).click()
    dialog = f.page.getByRole('dialog')
    await waitState(async () => await dialog.getByRole('checkbox', { name: '角色 售后主管', exact: true }).isChecked(), 'Saved role survives reopening')
    await dialog.getByRole('button', { name: '取消', exact: true }).click()
    await accountRow(f).getByRole('button', { name: '用户详情', exact: true }).click()
    dialog = f.page.getByRole('dialog')
    await dialog.getByRole('button', { name: '编辑', exact: true }).click()
    await f.page.getByRole('dialog').getByRole('heading', { name: /^编辑用户/ }).waitFor()
    assert.equal(f.state.writes.length, 1, 'Opening details and its edit entry never writes')
  })
  await scenario('customer profiles remain editable without employee operations at 390px', { customer: true, mobile: true }, async f => {
    assert.equal(await accountRow(f).getByRole('button', { name: '员工成员批量维护', exact: true }).count(), 0)
    assert.equal(await accountRow(f).getByRole('button', { name: '分配角色', exact: true }).count(), 0)
    const dialog = await openEdit(f)
    await dialog.getByRole('textbox', { name: /^姓名/ }).fill('更新后的客户')
    await f.page.screenshot({ path: path.join(reportDirectory, 'customer-profile-mobile.png'), fullPage: true })
    await save(f, dialog, '保存更改')
    assert.equal(f.state.account.realName, '更新后的客户')
    assert(f.state.requests.filter(item => item.pathname === '/api/users').every(item => item.params.memberType === 'CUSTOMER'))
    await accountRow(f).getByRole('button', { name: '用户详情', exact: true }).click()
    await f.page.getByRole('dialog').getByRole('button', { name: '编辑账号', exact: true }).click()
    await f.page.getByRole('dialog').getByRole('heading', { name: /^编辑用户/ }).waitFor()
    assert.equal(f.state.writes.length, 1)
  })
  await scenario('adding an employee retains exact existing-account binding', {}, async f => {
    await waitState(async () => !await f.page.getByRole('button', { name: '添加员工', exact: true }).isDisabled(), 'Member creation capabilities load')
    await f.page.getByRole('button', { name: '添加员工', exact: true }).click()
    const dialog = f.page.getByRole('dialog')
    await dialog.getByRole('heading', { name: '添加员工', exact: true }).waitFor()
    await dialog.getByRole('textbox', { name: '检索既有账号', exact: true }).fill('editable')
    await dialog.getByRole('button').filter({ hasText: '@editable-user' }).click()
    await dialog.getByTestId('selected-account-id').waitFor()
    assert.equal(await dialog.getByTestId('selected-account-id').innerText(), ids.account)
    assert.equal(await dialog.getByRole('textbox', { name: /^初始密码/ }).count(), 0)
    assert.equal(f.state.writes.length, 0, 'Binding selection never creates a global account')
  })
  await fs.writeFile(path.join(reportDirectory, 'report.json'), JSON.stringify({ status: 'PASS', scenarios, scope: 'Production React/Vite pages with isolated HTTP fixtures; no backend, database, or Docker resources' }, null, 2) + '\n')
  console.log(`PASS user editing browser: ${scenarios.length} scenarios; report ${reportDirectory}`)
} catch (error) {
  await fs.mkdir(reportDirectory, { recursive: true })
  await fs.writeFile(path.join(reportDirectory, 'report.json'), JSON.stringify({ status: 'FAIL', scenarios, error: error.message, startupOutput }, null, 2) + '\n')
  throw error
} finally {
  await browser?.close()
  if (vite && vite.exitCode === null) {
    vite.kill('SIGTERM')
    await new Promise(resolve => { vite.once('exit', resolve); setTimeout(resolve, 5000).unref() })
  }
}
