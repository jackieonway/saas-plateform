/** Actual Vite/React -> real owned Axum/PostgreSQL. No HTTP response stubs. */
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { createHash, randomUUID } from 'node:crypto'
import { mkdirSync, readFileSync } from 'node:fs'
import net from 'node:net'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { launchBrowser, browserName, playwrightVersion } from './browser/runtime.mjs'
import { holdRealResponse, installRaceTransport, raceCheckIds, runGrantRaces } from './phase15-data-grants-races.mjs'
import { startOwnedWorkOrders } from './phase15-work-orders-fixture.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const chunks = []
for await (const chunk of process.stdin) chunks.push(chunk)
const control = JSON.parse(Buffer.concat(chunks).toString('utf8'))
assert.equal(control.schemaVersion, 1)
const fixture = control.browser
assert.equal(fixture.schemaVersion, 1)
assert.match(fixture.apiOrigin, /^http:\/\/127\.0\.0\.1:[0-9]+$/)
const CHECK_IDS = [
  'root-empty-default-deny', 'preview-readonly', 'save-self-real-business',
  'scope-update-preserves-uuid', 'same-scope-noop', 'stale-version-conflict',
  'audit-request-correlation', 'revoke-next-business-request', 'readonly-root-no-write',
  'organization-admin-ui-and-forged403', 'negative-target-capability-matrix',
  'independent-read-route-guard',
  ...raceCheckIds,
]
const checks = [], requestIds = [], openContexts = []
const views = []
let browser, vite, origin, startup = '', spawnError, workOrders, screenshot
const delay = ms => new Promise(resolve => setTimeout(resolve, ms))
async function wait(predicate, message, timeout = 20_000) {
  const deadline = Date.now() + timeout
  while (Date.now() < deadline) {
    if (await predicate()) return
    await delay(20)
  }
  assert(await predicate(), message)
}
const check = (id) => {
  assert(CHECK_IDS.includes(id) && !checks.some(check => check.id === id), 'Unexpected or duplicate live check')
  checks.push({ id, status: 'PASS' })
  console.log('PASS phase15 live ' + id)
}
const pathFor = target => '/api/data-grants?' + new URLSearchParams(target)
const edit = (target, version, scope, action = 'read') => ({ target, expectedVersion: version,
  upsert: scope ? [{ resource: 'ticket', action, scope }] : [],
  remove: scope ? [] : [{ resource: 'ticket', action }],
})
async function api(token, pathname, method = 'GET', body) {
  const response = await fetch(origin + pathname, { method, headers: {
    Authorization: 'Bearer ' + token, 'Content-Type': 'application/json', 'x-request-id': randomUUID(),
  }, ...(body === undefined ? {} : { body: JSON.stringify(body) }) })
  const value = await response.json()
  return { status: response.status, body: value, requestId: response.headers.get('x-request-id') }
}
function requireStatus(response, expected, message) {
  assert.equal(response.status, expected, message)
  return response.body
}
async function snapshot() {
  return requireStatus(await api(fixture.sessions.admin, pathFor(fixture.target)), 200, 'Real grant snapshot succeeds')
}
async function startVite() {
  const port = await new Promise((resolve, reject) => {
    const server = net.createServer().on('error', reject)
    server.listen(0, '127.0.0.1', () => {
      const port = server.address().port
      server.close(error => error ? reject(error) : resolve(port))
    })
  })
  origin = 'http://127.0.0.1:' + port
  vite = spawn(process.execPath, [path.join(root, 'frontend/node_modules/vite/bin/vite.js'),
    '--host', '127.0.0.1', '--port', String(port), '--strictPort'], {
    cwd: path.join(root, 'frontend'), windowsHide: true,
    env: { ...process.env, BACKEND_PROXY_TARGET: fixture.apiOrigin },
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  vite.on('error', error => { spawnError = error })
  for (const stream of [vite.stdout, vite.stderr]) stream.on('data', data => {
    startup = (startup + data).slice(-6000)
  })
  await wait(() => {
    if (spawnError || vite.exitCode !== null) throw new Error('Owned Vite failed to start')
    return startup.includes('ready in')
  }, 'Owned Vite readiness', 30_000)
}
async function open(token, query = '', options = {}) {
  const context = await browser.newContext({ viewport: { width: 1540, height: 1200 } })
  openContexts.push(context)
  await context.addInitScript(({ token, system }) => {
    localStorage.setItem('platform-session-v1', JSON.stringify({ schema: 1, version: 1,
      sessionId: crypto.randomUUID(), status: 'active', token, refreshToken: null }))
    localStorage.setItem('platform-current-app', system)
    localStorage.setItem('platform-theme', 'light')
  }, { token, system: fixture.systemApplicationId })
  if (options.raceTransport) await context.addInitScript(installRaceTransport)
  // Realtime transport is unrelated to grant management. HTTP remains real.
  await context.routeWebSocket(() => true, () => {})
  const page = await context.newPage()
  page.setDefaultTimeout(20_000)
  const errors = []
  const diagnostic = { userFilterPresent: new URLSearchParams(query).has('userId'), optionRequests: 0, options: [] }
  views.push({ page, diagnostic })
  page.on('request', request => {
    if (new URL(request.url()).pathname === '/api/data-grants/options') diagnostic.optionRequests++
  })
  page.on('response', async response => {
    const url = new URL(response.url())
    if (url.pathname !== '/api/data-grants/options' || diagnostic.options.length >= 20) return
    const body = await response.json().catch(() => null)
    const records = Array.isArray(body?.records) ? body.records : []
    diagnostic.options.push({ status: response.status(),
      code: typeof body?.code === 'string' && /^[A-Z0-9_]{1,64}$/.test(body.code) ? body.code : null,
      userFilterPresent: url.searchParams.has('userId'), applicationFilterPresent: url.searchParams.has('applicationId'),
      applicationsArray: Array.isArray(body?.applications), applicationCount: Array.isArray(body?.applications) ? body.applications.length : null,
      recordsArray: Array.isArray(body?.records), recordCount: records.length,
      writableCount: records.filter(record => record?.canWrite === true).length,
      readonlyCount: records.filter(record => record?.canWrite === false).length,
      total: Number.isSafeInteger(body?.total) ? body.total : null })
  })
  page.on('pageerror', error => errors.push(error.message))
  await page.goto(origin + '/system/data-grants' + query, { waitUntil: 'domcontentloaded' })
  await page.getByRole('complementary', { name: '应用导航', exact: true }).waitFor()
  return { page, context, errors }
}
async function chooseApplication(page) {
  await page.getByRole('combobox', { name: '选择数据授权应用', exact: true }).click()
  await page.getByRole('option').filter({ hasText: fixture.application.name }).click()
}
async function chooseMember(page) {
  const member = page.getByTestId('grant-member-' + fixture.target.membershipId)
  await member.waitFor()
  await member.click()
  await page.getByTestId('grant-current-snapshot').waitFor()
  await page.getByRole('combobox', { name: 'ticket/read 数据范围', exact: true }).waitFor()
}
async function chooseScope(page, scope) {
  await page.getByRole('combobox', { name: 'ticket/read 数据范围', exact: true }).click()
  const names = { SELF: 'SELF · 本人', ORG: 'ORG · 当前组织', NONE: '不授权' }
  await page.getByRole('option', { name: names[scope], exact: true }).click()
}
function mutationResponses(page) {
  const replies = []
  page.on('response', response => {
    if (new URL(response.url()).pathname === '/api/data-grants' && response.request().method() === 'PATCH') replies.push(response)
  })
  return replies
}
async function preview(page, expected = 200) {
  const response = page.waitForResponse(response => new URL(response.url()).pathname === '/api/data-grants/preview'
    && response.request().method() === 'POST')
  await page.getByRole('button', { name: '预览变更', exact: true }).click()
  const reply = await response
  assert.equal(reply.status(), expected, 'UI preview uses actual controlled API')
  return reply.json()
}
async function save(page) {
  const response = page.waitForResponse(response => new URL(response.url()).pathname === '/api/data-grants'
    && response.request().method() === 'PATCH')
  await page.getByRole('button', { name: '确认保存', exact: true }).click()
  const reply = await response
  assert.equal(reply.status(), 200, 'UI save succeeds against real API')
  const id = reply.headers()['x-request-id']
  assert.match(id || '', /^[0-9a-f-]{36}$/)
  requestIds.push(id)
  await page.getByTestId('grant-save-request-id').filter({ hasText: id }).waitFor()
  const snapshot = await reply.json()
  await wait(async () => await page.getByTestId('grant-current-snapshot').getAttribute('data-version') === snapshot.version,
    'Saved UI snapshot observes the committed response version')
  return snapshot
}
async function business(token) {
  const context = await api(token, '/api/apps/' + fixture.application.code + '/context')
  const response = await fetch(workOrders.origin + '/work-orders-api/tickets', {
    headers: { Authorization: 'Bearer ' + token, 'x-request-id': randomUUID() },
  })
  const tickets = { status: response.status, body: await response.json() }
  return { context, tickets }
}

try {
  workOrders = await startOwnedWorkOrders(fixture, control.workOrders)
  await startVite()
  browser = await launchBrowser()
  const browserVersion = await browser.version()
  console.log('PHASE15_LIVE_SOURCE ' + JSON.stringify({ mode: 'actual-vite-real-api-owned-postgres',
    product: browserName, browserVersion, playwrightVersion,
    testSha256: createHash('sha256').update(readFileSync(fileURLToPath(import.meta.url))).digest('hex') }))

  await runGrantRaces({ fixture, open, chooseApplication, chooseScope, preview, snapshot, wait, check })

  const rootView = await open(fixture.sessions.admin, '?userId=' + fixture.target.userId)
  const { page } = rootView
  await page.getByRole('combobox', { name: '选择数据授权应用', exact: true }).waitFor()
  assert.equal(await page.getByRole('combobox', { name: 'ticket/read 数据范围', exact: true }).count(), 0,
    'No member or capability is selected before explicit discovery')
  await chooseApplication(page)
  await chooseMember(page)
  const initial = await snapshot()
  assert.deepEqual(initial.grants, [], 'Direct membership starts with no default grant')
  const issued = requireStatus(await api(fixture.sessions.business, '/api/oauth/session-token', 'POST', {
    appCode: fixture.application.code,
  }), 201, 'Business token comes from the actual issuer')
  const businessToken = issued.accessToken
  assert.equal(typeof businessToken, 'string')
  const denied = await business(businessToken)
  assert.equal(denied.context.status, 200)
  assert.deepEqual(denied.context.body.dataGrants, [])
  assert.equal(denied.tickets.status, 403)
  assert(!denied.context.body.permissions.some(permission => permission.startsWith('system:')),
    'Business user has no management permission')
  check('root-empty-default-deny')

  const writes = mutationResponses(page)
  await chooseScope(page, 'SELF')
  const proposed = await preview(page)
  assert.equal(proposed.after[0].scope, 'SELF')
  assert.deepEqual(await snapshot(), initial, 'Preview leaves persisted version/grants unchanged')
  assert.equal(writes.length, 0, 'Preview makes no PATCH')
  if (process.env.PHASE15_SCREENSHOT_PATH) {
    const destination = path.resolve(process.env.PHASE15_SCREENSHOT_PATH)
    assert(path.isAbsolute(process.env.PHASE15_SCREENSHOT_PATH), 'Screenshot must use an explicit report-only absolute path')
    assert(!destination.startsWith(root + path.sep) || destination.startsWith(path.join(root, 'target') + path.sep),
      'Screenshot cannot overwrite managed source inputs')
    mkdirSync(path.dirname(destination), { recursive: true })
    await page.screenshot({ path: destination, fullPage: true })
    screenshot = { path: destination, sha256: createHash('sha256').update(readFileSync(destination)).digest('hex') }
  }
  check('preview-readonly')
  const self = await save(page)
  const grantId = self.grants[0].id
  assert.equal(self.grants[0].scope, 'SELF')
  const persistedSelf = await snapshot()
  assert.equal(persistedSelf.grants[0].id, grantId)
  const selfBusiness = await business(businessToken)
  assert.equal(selfBusiness.context.body.dataGrants[0].scope, 'SELF')
  assert.notEqual(selfBusiness.context.body.contextVersion, denied.context.body.contextVersion)
  assert.equal(selfBusiness.tickets.status, 200)
  assert.equal(selfBusiness.tickets.body.total, 1)
  assert.deepEqual(selfBusiness.tickets.body.records.map(record => record.id), [workOrders.ids.own],
    'SELF returns only the actual business record owned by the token subject')
  check('save-self-real-business')

  await chooseScope(page, 'ORG')
  await preview(page)
  const organization = await save(page)
  assert.equal(organization.grants[0].scope, 'ORG')
  assert.equal(organization.grants[0].id, grantId, 'Scope update preserves stored grant UUID')
  assert.equal(organization.target.membershipId, fixture.target.membershipId)
  const orgBusiness = await business(businessToken)
  assert.equal(orgBusiness.context.body.dataGrants[0].scope, 'ORG')
  assert.equal(orgBusiness.tickets.status, 200)
  assert.equal(orgBusiness.tickets.body.total, 2)
  assert.deepEqual(orgBusiness.tickets.body.records.map(record => record.id).sort(), [workOrders.ids.own, workOrders.ids.peer].sort(),
    'ORG adds the peer owner record within the selected organization')
  assert(!selfBusiness.tickets.body.records.some(record => record.id === workOrders.ids.outside)
    && !orgBusiness.tickets.body.records.some(record => record.id === workOrders.ids.outside),
  'Neither SELF nor ORG returns the sibling-organization record, even with the same owner')
  check('scope-update-preserves-uuid')

  await chooseScope(page, 'ORG')
  await page.getByText('没有实际变更，无需保存', { exact: true }).waitFor()
  assert.equal(await page.getByRole('button', { name: '确认保存', exact: true }).count(), 0)
  assert.deepEqual(await snapshot(), organization, 'Same scope does not change grant/version')
  assert.equal(writes.length, 2)
  check('same-scope-noop')

  // The actual page queries the exact successful request, under system:audit.
  const auditResponse = page.waitForResponse(response => {
    const url = new URL(response.url())
    return url.pathname === '/api/audit-logs' && url.searchParams.get('request_id') === requestIds[1]
      && url.searchParams.get('application_id') === fixture.application.id
  })
  await page.getByRole('button', { name: '查询本次审计', exact: true }).click()
  const auditReply = await auditResponse
  assert.equal(auditReply.status(), 200, 'Actual audit button uses the independently authorized API')
  const auditBody = await auditReply.json()
  assert.equal(auditBody.records.length, 1)
  assert.equal(auditBody.records[0].requestId, requestIds[1])
  const readerAudit = await api(fixture.sessions.reader, '/api/audit-logs')
  assert.equal(readerAudit.status, 403, 'Grant read permission does not imply audit permission')
  check('audit-request-correlation')

  // A second real manager changes the current version while this editor holds
  // its previously observed snapshot. The conflict is produced by PostgreSQL.
  const concurrent = await api(fixture.sessions.admin, '/api/data-grants', 'PATCH',
    edit(fixture.target, organization.version, 'SELF'))
  requireStatus(concurrent, 200, 'Competing controlled API change succeeds')
  requestIds.push(concurrent.requestId)
  await chooseScope(page, 'SELF')
  await preview(page, 409)
  await page.getByRole('alert').filter({ hasText: '授权版本已变化，请重新读取并审阅' }).waitFor()
  assert.equal(await page.getByRole('button', { name: '确认保存', exact: true }).count(), 0)
  const reconciliation = await holdRealResponse(page, { method: 'GET', pathname: '/api/data-grants', member: fixture.target.membershipId })
  await page.getByRole('button', { name: '重新读取授权', exact: true }).click()
  assert.equal(await reconciliation.ready, 200)
  assert.equal(await page.getByRole('button', { name: '预览变更', exact: true }).count(), 0,
    'Conflict reconciliation must complete before a new preview')
  assert.equal(await page.getByRole('button', { name: '确认保存', exact: true }).count(), 0)
  await reconciliation.release()
  await wait(async () => await page.getByTestId('grant-current-snapshot').getAttribute('data-version') === concurrent.body.version,
    'Explicit reread replaces stale editor snapshot')
  check('stale-version-conflict')

  await chooseScope(page, 'NONE')
  await preview(page)
  const revoked = await save(page)
  assert.deepEqual(revoked.grants, [])
  const afterRevoke = await business(businessToken)
  assert.deepEqual(afterRevoke.context.body.dataGrants, [])
  assert.notEqual(afterRevoke.context.body.contextVersion, selfBusiness.context.body.contextVersion)
  assert.equal(afterRevoke.tickets.status, 403, 'Same real app token loses actual work-order access on the next request')
  check('revoke-next-business-request')

  const readonly = await open(fixture.sessions.reader, '?userId=' + fixture.target.userId)
  await chooseApplication(readonly.page)
  await readonly.page.getByTestId('grant-member-' + fixture.target.membershipId).click()
  await readonly.page.getByTestId('grant-current-snapshot').waitFor()
  assert.equal(await readonly.page.getByRole('combobox', { name: 'ticket/read 数据范围', exact: true }).count(), 0)
  assert.equal(await readonly.page.getByRole('button', { name: '预览变更', exact: true }).count(), 0)
  assert.equal(await readonly.page.getByRole('button', { name: '确认保存', exact: true }).count(), 0)
  const readDenied = await api(fixture.sessions.reader, '/api/data-grants', 'PATCH', edit(fixture.target, revoked.version, 'ORG'))
  assert.equal(readDenied.status, 403)
  check('readonly-root-no-write')

  const orgView = await open(fixture.sessions.organizationAdmin, '?userId=' + fixture.target.userId)
  await chooseApplication(orgView.page)
  await orgView.page.getByTestId('grant-member-' + fixture.target.membershipId).click()
  await orgView.page.getByTestId('grant-current-snapshot').waitFor()
  assert.equal(await orgView.page.getByRole('combobox', { name: 'ticket/read 数据范围', exact: true }).count(), 0)
  assert.equal(await orgView.page.getByRole('button', { name: '预览变更', exact: true }).count(), 0)
  const forged = await api(fixture.sessions.organizationAdmin, '/api/data-grants', 'PATCH', edit(fixture.target, revoked.version, 'ORG'))
  assert.equal(forged.status, 403, 'Actual organization admin with independent write still needs platform-root context')
  check('organization-admin-ui-and-forged403')

  const options = requireStatus(await api(fixture.sessions.admin, '/api/data-grants/options?' + new URLSearchParams({
    applicationId: fixture.application.id, page: '1', size: '50',
  })), 200, 'Real discovery is independently authorized')
  assert(options.records.some(row => row.target.membershipId === fixture.target.membershipId))
  assert(!options.records.some(row => [fixture.inactiveTarget.membershipId, fixture.outsideTarget.membershipId].includes(row.target.membershipId)),
    'Invalid and out-of-bound memberships are absent from selection')
  for (const target of [fixture.inactiveTarget, fixture.outsideTarget]) {
    const rejected = await api(fixture.sessions.admin, '/api/data-grants', 'PATCH', edit(target, revoked.version, 'ORG'))
    assert([403, 404].includes(rejected.status), 'Forged inactive/out-of-bound target is rejected by actual API')
  }
  const unknown = await api(fixture.sessions.admin, '/api/data-grants', 'PATCH', edit(fixture.target, revoked.version, 'ORG', 'delete'))
  assert.equal(unknown.status, 400)
  assert.deepEqual(await snapshot(), revoked, 'All real negative requests leave saved state untouched')
  check('negative-target-capability-matrix')

  const noRead = await open(fixture.sessions.noRead)
  await noRead.page.getByText('无访问权限', { exact: true }).waitFor()
  assert.equal(await noRead.page.getByRole('combobox', { name: '选择数据授权应用', exact: true }).count(), 0)
  assert.equal((await api(fixture.sessions.noRead, '/api/data-grants/options')).status, 403)
  check('independent-read-route-guard')

  for (const view of [rootView, readonly, orgView, noRead]) assert.deepEqual(view.errors, [], 'Actual React renders without uncaught errors')
  assert.deepEqual([...checks.map(check => check.id)].sort(), [...CHECK_IDS].sort())
  assert.equal(requestIds.length, 4)
  assert.equal(new Set(requestIds).size, requestIds.length)
  console.log('PHASE15_LIVE_RESULT ' + JSON.stringify({ schemaVersion: 1, product: browserName,
    browserVersion, playwrightVersion, userAgent: await page.evaluate(() => navigator.userAgent),
    vendor: await page.evaluate(() => navigator.vendor), checks, passed: checks.length, failed: 0, skipped: 0, requestIds,
    workOrders: { selfCount: 1, orgCount: 2, revokedStatus: 403, sameToken: true,
      independentDatabase: true, outsideNeverReturned: true }, ...(screenshot ? { screenshot } : {}) }))
} catch (error) {
  // Do not dump fixture, headers, localStorage, token responses or page source.
  const current = views.at(-1)
  if (current) {
    const state = await (async () => {
      const combobox = current.page.getByRole('combobox', { name: '选择数据授权应用', exact: true })
      return { comboboxCount: await combobox.count(), disabled: await combobox.isDisabled().catch(() => null),
        invalidUserFilter: await current.page.getByText('用户筛选无效，请从成员管理重新选择目标。', { exact: true }).count() > 0 }
    })().catch(() => ({ pageClosed: true }))
    console.error('PHASE15_LIVE_DIAGNOSTIC ' + JSON.stringify({ ...current.diagnostic, ...state }))
  }
  console.error('Phase15 live browser failure: ' + error.message)
  process.exitCode = 1
} finally {
  for (const context of openContexts.reverse()) await context.close().catch(() => {})
  if (browser) await browser.close().catch(() => {})
  if (workOrders) await workOrders.close().catch(() => { process.exitCode = 1 })
  if (vite && vite.exitCode === null) {
    vite.kill('SIGTERM')
    await Promise.race([new Promise(resolve => vite.once('close', resolve)), delay(3000)])
    if (vite.exitCode === null) vite.kill('SIGKILL')
  }
}
