/** Actual production React/Vite and Vue/Vite hosts, three Rust services and three owned databases. */
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { launchBrowser, browserName, playwrightVersion } from './browser/runtime.mjs'
import { startPhase27Services, APP, API, delay, wait } from './helpers/phase27-stores-services.mjs'
import { storesBrowserChecks as checksRequired } from '../scripts/ci/stores-browser-results.mjs'

const sha = bytes => createHash('sha256').update(bytes).digest('hex')
const checks = [], evidence = {}, contexts = [], operations = [], assets = [], errors = [], native = new Map(), headerTasks = new Set(), faults = []
let services, browser, browserClosed = false, failure, cleanup
function check(id, proof) {
  assert(checksRequired.includes(id) && !checks.some(check => check.id === id)); checks.push({ id, status: 'PASS' }); evidence[id] = proof
  if (process.env.STORES_BROWSER_PROGRESS_FILE) writeFileSync(process.env.STORES_BROWSER_PROGRESS_FILE, JSON.stringify({ evidenceClass: 'PROGRESS_ONLY', checks: checks.map(item => item.id), lastCheck: id, at: new Date().toISOString() }) + '\n')
  console.log('PASS phase27 browser ' + id)
}
const frameFor = page => page.frameLocator('.micro-mount-zone iframe')
const form = (view, kind) => view.frame.locator(`form[data-form-kind="${kind}"]`)
async function navigate(view, pathname) {
  await view.page.evaluate(next => { history.pushState(null, '', next); dispatchEvent(new PopStateEvent('popstate')) }, pathname)
  await view.page.waitForURL(url => url.pathname === pathname)
}
async function active(view) {
  await view.page.getByTestId('micro-status-bar').getByText('运行中', { exact: true }).waitFor()
  await view.frame.getByTestId(view.appCode === 'stores' ? 'stores-app' : 'work-orders-app').waitFor()
}
async function install(context, actor) {
  await context.exposeBinding('__phase27Fetch', (_source, value) => {
    const record = native.get(value.requestId) || { requestId: value.requestId }
    native.set(value.requestId, record)
    if (value.phase === 'start') { assert(!record.started); Object.assign(record, value, { started: true }) }
    else if (value.phase === 'abort') record.signalAborted = true
    else { assert(!record.nativeOutcome); Object.assign(record, value) }
  })
  await context.addInitScript(() => {
    if (window.top === window) return
    const fetch = window.fetch.bind(window)
    window.fetch = function (input, options) {
      let entry, signal, aborted
      const emit = value => { try { void window.__phase27Fetch(value).catch(() => {}) } catch {} }
      try {
        const url = new URL(typeof input === 'string' || input instanceof URL ? input : input.url, location.href)
        if (/^\/(?:stores|work-orders)-api\//.test(url.pathname)) {
          const headers = new Headers(options?.headers || (input instanceof Request ? input.headers : undefined))
          signal = options?.signal || (input instanceof Request ? input.signal : undefined)
          entry = { requestId: headers.get('x-request-id'), method: options?.method || (input instanceof Request ? input.method : 'GET'), path: url.pathname,
            credentials: options?.credentials || (input instanceof Request ? input.credentials : 'same-origin'), cookiePresent: headers.has('cookie'), idempotencyKeyPresent: headers.has('idempotency-key'),
            bearerPresent: /^Bearer .+/.test(headers.get('authorization') || ''), signalAborted: Boolean(signal?.aborted) }
          emit({ phase: 'start', ...entry }); aborted = () => emit({ phase: 'abort', requestId: entry.requestId, signalAborted: true })
          signal?.addEventListener('abort', aborted, { once: true })
        }
      } catch {}
      const result = fetch(input, options)
      if (entry) result.then(() => emit({ phase: 'settled', requestId: entry.requestId, nativeOutcome: 'fulfilled', signalAborted: Boolean(signal?.aborted) }),
        error => emit({ phase: 'settled', requestId: entry.requestId, nativeOutcome: 'rejected', nativeErrorName: error?.name || 'Error', signalAborted: Boolean(signal?.aborted) }))
        .finally(() => signal?.removeEventListener('abort', aborted))
      return result
    }
  })
  await context.addCookies([{ name: 'phase27-cookie-must-not-reach-business', value: 'opaque-observer', url: services.origin, httpOnly: true, secure: true }])
  await context.addInitScript(({ session, refreshToken, applicationId }) => {
    if (window.top !== window || sessionStorage.getItem('phase27-session-seeded')) return
    // Only this actual host stores its real session. The opaque micro applications cannot access it.
    if (!localStorage.getItem('platform-session-v1')) localStorage.setItem('platform-session-v1', JSON.stringify({ schema: 1, version: 1, sessionId: crypto.randomUUID(), status: 'active', token: session, refreshToken }))
    if (!localStorage.getItem('platform-current-app')) localStorage.setItem('platform-current-app', applicationId)
    localStorage.setItem('platform-theme', 'light'); sessionStorage.setItem('phase27-session-seeded', 'true')
  }, { session: actor.session, refreshToken: actor.sessionResponse.refreshToken, applicationId: services.apps[actor.appCode || 'stores'].application.id })
}
async function pageIn(context, appCode, pathname) {
  const page = await context.newPage(), issuances = []; page.setDefaultTimeout(25000)
  page.on('pageerror', error => errors.push(error.message))
  page.on('request', request => {
    const url = new URL(request.url())
    if (!/^\/(?:stores|work-orders)-api\//.test(url.pathname)) return
    const initial = request.headers()
    const operation = { method: request.method(), path: url.pathname, query: url.search, outcome: null, responseStatus: null,
      requestId: initial['x-request-id'], origin: initial.origin ?? null, cookiePresent: !!initial.cookie, bearerPresent: /^Bearer .+/.test(initial.authorization || '') }
    operations.push(operation)
    const task = request.allHeaders().then(headers => {
      operation.requestId = headers['x-request-id']; operation.origin = headers.origin ?? null; operation.cookiePresent = !!headers.cookie; operation.bearerPresent = /^Bearer .+/.test(headers.authorization || '')
      try { operation.frameType = request.frame() === page.mainFrame() ? 'host' : 'iframe'; operation.framePath = new URL(request.frame().url()).pathname } catch { operation.frameType = 'retired' }
    }); headerTasks.add(task); task.then(() => headerTasks.delete(task), () => { headerTasks.delete(task); operation.headersUnavailable = true })
    request.__phase27 = operation
  })
  page.on('requestfinished', request => { if (request.__phase27) request.__phase27.outcome = 'finished' })
  page.on('requestfailed', request => { if (request.__phase27) { request.__phase27.outcome = 'failed'; request.__phase27.failureCode = request.failure()?.errorText } })
  page.on('response', response => {
    if (response.request().__phase27) response.request().__phase27.responseStatus = response.status()
    const url = new URL(response.url())
    if (url.pathname === '/api/oauth/session-token' && response.request().method() === 'POST') {
      try { if (JSON.parse(response.request().postData() || '{}').appCode === appCode) issuances.push({ appCode, status: response.status(), at: Date.now() }) } catch {}
    }
    if (/^\/sub-apps\/(?:stores|work-orders)\/versions\/phase27\//.test(url.pathname)) assets.push({ path: url.pathname, status: response.status(), contentType: response.headers()['content-type'], allowOrigin: response.headers()['access-control-allow-origin'] })
  })
  await page.goto(services.origin + pathname, { waitUntil: 'domcontentloaded' })
  return { page, context, frame: frameFor(page), appCode, issuances }
}
async function open(actor, pathname = '/micro/stores', appCode = 'stores') {
  const context = await browser.newContext({ viewport: { width: 1540, height: 1120 }, acceptDownloads: true }); contexts.push(context)
  await install(context, actor); return pageIn(context, appCode, pathname)
}
async function business(actor, endpoint = API, options = {}, expected = 200) {
  const result = await services.businessCall(actor, endpoint, options)
  assert.equal(result.status, expected, 'Actual Rust ' + (options.method || 'GET') + ' ' + endpoint + ': ' + result.data?.code)
  return result
}
async function clicked(view, kind, button, endpoint, method, expected) {
  const pending = view.page.waitForResponse(response => new URL(response.url()).pathname === endpoint && response.request().method() === method)
  await form(view, kind).getByRole('button', { name: button, exact: true }).click()
  const response = await pending; assert.equal(response.status(), expected)
  // The SDK intentionally cancels denied response bodies as soon as 401/403
  // headers arrive. Their HTTP status remains authoritative even when CDP has no body.
  return [401, 403].includes(expected) ? { status: expected } : response.json()
}
async function screenshot(view, name) {
  const directory = process.env.STORES_BROWSER_REPORT_DIR; assert(path.isAbsolute(directory)); mkdirSync(directory, { recursive: true })
  const destination = path.join(directory, name + '.png'); await view.page.screenshot({ path: destination, fullPage: true })
  ;(evidence.screenshots ||= []).push({ path: destination, sha256: sha(readFileSync(destination)) })
}
async function dropCommitted(view, endpoint, method, action) {
  let fault
  const handler = async route => {
    if (route.request().method() !== method || new URL(route.request().url()).pathname !== endpoint || fault) return route.continue()
    const headers = route.request().headers(), response = await route.fetch(), body = await response.json()
    assert([200, 201].includes(response.status())); assert(body.id)
    fault = { requestId: headers['x-request-id'], forwardedRequestId: headers['x-request-id'], method, path: endpoint,
      actualUpstreamStatus: response.status(), actualUpstreamCommitted: false, automaticRetryCount: 0, body }
    await route.abort('connectionreset')
  }
  await view.context.route('**/*', handler)
  try { await action(); await wait(() => !!fault, 'Actual upstream write response before drop'); return fault }
  finally { await view.context.unroute('**/*', handler) }
}
const date = offset => { const now = new Date(); now.setUTCDate(now.getUTCDate() + offset); return new Intl.DateTimeFormat('en-CA', { timeZone: 'Asia/Shanghai', year: 'numeric', month: '2-digit', day: '2-digit' }).format(now) }
try {
  services = await startPhase27Services(); browser = await launchBrowser()
  const actor = services.actors.ordinary, peer = services.actors.peer
  const view = await open(actor, '/micro/stores/store-create'); await active(view)
  await screenshot(view, 'phase27-real-create-initial')
  const started = Date.now()
  await form(view, 'store-create').waitFor()
  const iframe = view.page.locator('.micro-mount-zone iframe'), sandbox = await iframe.getAttribute('sandbox')
  assert(sandbox.includes('allow-scripts') && !sandbox.includes('allow-same-origin'))
  const manifest = JSON.parse(readFileSync(path.join(services.ownedRoot, 'stores-app/build-info.json'), 'utf8'))
  await wait(() => [manifest.entry, ...manifest.dynamicChunks].every(asset => assets.some(item => item.path.endsWith('/' + asset) && item.status === 200 && ['*', 'null'].includes(item.allowOrigin))),
    'Actual opaque iframe ESM entry and every dynamic chunk load with anonymous CORS')
  check('production-react-vite-independent-build', { sandbox, moduleEntry: manifest.entry, dynamicChunks: manifest.dynamicChunks, build: services.source.separateFrontend })
  assert.equal(services.menus.size, 11)
  assert.equal((await services.platformDb.query('SELECT is_super_admin FROM users WHERE id=$1', [actor.userId])).rows[0].is_super_admin, false)
  check('ordinary-user-authorization-menu11', { userId: actor.userId, applicationId: services.application.id, menus: services.menus.size })

  const create = form(view, 'store-create')
  await create.getByLabel('门店名称').fill('Phase27 实际普通用户门店')
  await create.getByLabel('业务联系信息').fill('真实平台、独立门店库和精确金额')
  await create.getByLabel('成员搜索').fill('peer'); await create.getByRole('button', { name: '搜索成员', exact: true }).click()
  await create.getByRole('button', { name: '选择 Phase27 peer', exact: true }).click()
  assert.equal(await create.getByLabel('负责人用户ID').inputValue(), peer.userId)
  let store = await clicked(view, 'store-create', '创建门店', API, 'POST', 201)
  assert.equal(store.managerUserId, peer.userId); assert.equal(store.organizationId, actor.organizationId)
  check('store-create-manager-directory', { storeId: store.id, managerId: peer.userId, directory: '/stores-api/members?resource=store&action=create' })

  await navigate(view, '/micro/stores/stores/' + store.id); await form(view, 'store-edit').getByLabel('门店名称').waitFor()
  await form(view, 'store-edit').getByLabel('门店名称').fill('Phase27 更新资料')
  await form(view, 'store-edit').getByLabel('营业状态').selectOption('CLOSED')
  store = await clicked(view, 'store-edit', '保存门店', API + '/' + store.id, 'PATCH', 200)
  assert.equal(store.businessStatus, 'CLOSED'); check('store-update-status', { id: store.id, version: store.version, businessStatus: store.businessStatus })
  const conflictDraft = 'Phase27 保留冲突输入'
  await form(view, 'store-edit').getByLabel('门店名称').fill(conflictDraft)
  store = (await business(actor, API + '/' + store.id, { method: 'PATCH', body: { version: store.version, businessContact: '并发正式API更新' } })).data
  const conflict = await clicked(view, 'store-edit', '保存门店', API + '/' + store.id, 'PATCH', 409); assert.equal(conflict.code, 'VERSION_CONFLICT')
  assert.equal(await form(view, 'store-edit').getByLabel('门店名称').inputValue(), conflictDraft)
  await form(view, 'store-edit').getByRole('button', { name: '读取最新版本', exact: true }).click()
  await form(view, 'store-edit').getByRole('button', { name: '使用最新版本', exact: true }).click()
  store = await clicked(view, 'store-edit', '保存门店', API + '/' + store.id, 'PATCH', 200)
  assert.equal(store.name, conflictDraft); check('version-conflict-explicit-rebase', { actualStatus: 409, latestVersion: store.version, preserved: conflictDraft })

  await navigate(view, '/micro/stores/staff-assign'); const assign = form(view, 'staff-assign')
  await assign.getByLabel('门店ID').fill(store.id); await assign.getByLabel('成员搜索').fill('peer')
  await assign.getByRole('button', { name: '搜索成员', exact: true }).click(); await assign.getByRole('button', { name: '选择 Phase27 peer', exact: true }).click()
  const staff = await clicked(view, 'staff-assign', '关联员工', API + '/' + store.id + '/staff', 'POST', 201)
  assert.equal(staff.userId, peer.userId)
  await navigate(view, '/micro/stores/staff'); await form(view, 'staff-query').getByLabel('门店ID').fill(store.id)
  await clicked(view, 'staff-query', '查询员工', API + '/' + store.id + '/staff', 'GET', 200)
  await view.frame.locator(`[data-row-id="${staff.id}"]`).waitFor()
  const removed = view.page.waitForResponse(response => new URL(response.url()).pathname === API + '/' + store.id + '/staff/' + staff.id && response.request().method() === 'DELETE')
  await view.frame.locator(`[data-row-id="${staff.id}"]`).getByRole('button', { name: '解除关联', exact: true }).click()
  assert.equal((await removed).status(), 200)
  assert.equal((await business(actor, API + '/' + store.id + '/staff')).data.total, 0)
  check('staff-association-unassignment', { linkId: staff.id, userId: staff.userId, created: 201, removed: 200 })

  const today = date(0), yesterday = date(-1), unknownDay = date(-2)
  await navigate(view, '/micro/stores/daily-report-create'); const reportCreate = form(view, 'report-create')
  assert.equal(await reportCreate.getByLabel('营业日期').inputValue(), today)
  await reportCreate.getByLabel('门店ID').fill(store.id); await reportCreate.getByLabel('营业日期').fill(today)
  await reportCreate.getByLabel('营业金额').fill('100.10'); await reportCreate.getByLabel('备注').fill('精确十进制文本')
  let daily = await clicked(view, 'report-create', '录入日报', API + '/' + store.id + '/daily-reports', 'POST', 201)
  assert.equal(daily.amount, '100.10')
  const duplicate = await clicked(view, 'report-create', '录入日报', API + '/' + store.id + '/daily-reports', 'POST', 409); assert.equal(duplicate.code, 'DAILY_REPORT_EXISTS')
  assert.equal(await reportCreate.getByLabel('营业金额').inputValue(), '100.10')
  await navigate(view, '/micro/stores/daily-reports/' + daily.id); await form(view, 'report-edit').getByLabel('营业金额').fill('200.20')
  daily = await clicked(view, 'report-edit', '保存日报', '/stores-api/daily-reports/' + daily.id, 'PATCH', 200)
  await navigate(view, '/micro/stores/daily-report-create'); await form(view, 'report-create').getByLabel('营业日期').fill(yesterday)
  await form(view, 'report-create').getByLabel('营业金额').fill('9999999999999999.99')
  await clicked(view, 'report-create', '录入日报', API + '/' + store.id + '/daily-reports', 'POST', 201)
  await navigate(view, '/micro/stores/reports'); const summary = form(view, 'summary')
  await summary.getByLabel('门店ID').fill(store.id); await summary.getByLabel('开始日期').fill(yesterday); await summary.getByLabel('结束日期').fill(today)
  const total = await clicked(view, 'summary', '查询汇总', '/stores-api/daily-reports/summary', 'GET', 200)
  assert.equal(total.totalAmount, '10000000000000200.19'); assert.equal(total.count, 2)
  await view.frame.getByTestId('server-summary').getByText(total.totalAmount, { exact: true }).waitFor(); await screenshot(view, 'phase27-real-ledger-light')
  check('daily-create-update-duplicate-summary', { dailyId: daily.id, duplicateCode: duplicate.code, explicitShanghaiDate: today, totalAmount: total.totalAmount, count: total.count })

  const child = await open(services.actors.child, '/micro/stores/staff'); await active(child)
  const childStart = operations.length
  await form(child, 'staff-query').getByLabel('门店ID').fill(store.id)
  await clicked(child, 'staff-query', '查询员工', API + '/' + store.id + '/staff', 'GET', 200)
  await navigate(child, '/micro/stores/reports'); await form(child, 'summary').getByLabel('门店ID').fill(store.id)
  await form(child, 'summary').getByLabel('开始日期').fill(yesterday); await form(child, 'summary').getByLabel('结束日期').fill(today)
  await clicked(child, 'summary', '查询汇总', '/stores-api/daily-reports/summary', 'GET', 200)
  assert(!operations.slice(childStart).some(item => item.path === API || item.path === API + '/' + store.id))
  assert.equal((await business(services.actors.child, API + '/' + store.id, {}, 403)).data.code, 'OPERATION_DENIED')
  check('staff-report-without-store-read', { directStoreId: store.id, staffStatus: 200, summaryStatus: 200, storeReadStatus: 403, silentStoreReadCount: 0 })

  await navigate(view, '/micro/stores/daily-report-create'); const draftForm = form(view, 'report-create')
  await draftForm.getByLabel('营业日期').fill(unknownDay); await draftForm.getByLabel('营业金额').fill('7.89')
  const formId = await draftForm.getAttribute('data-form-id'), requestsBeforeTheme = operations.length
  await view.page.getByRole('button', { name: '切换到深色模式', exact: true }).click()
  await view.frame.locator('[data-testid="stores-app"][data-theme="dark"]').waitFor()
  assert.equal(await draftForm.getAttribute('data-form-id'), formId); assert.equal(await draftForm.getByLabel('营业金额').inputValue(), '7.89')
  assert.equal(operations.length, requestsBeforeTheme); check('theme-keeps-draft-no-request', { formIdPreserved: true, amount: '7.89', extraBusinessRequests: 0 })
  const lost = await dropCommitted(view, API + '/' + store.id + '/daily-reports', 'POST', () => draftForm.getByRole('button', { name: '录入日报', exact: true }).click())
  const committed = (await services.businessDb.query('SELECT id,amount::text,business_date::text FROM store_daily_reports WHERE id=$1', [lost.body.id])).rows
  assert.equal(committed.length, 1); assert.equal(committed[0].business_date, unknownDay); lost.actualUpstreamCommitted = true
  await draftForm.getByText(/本次写入结果未知/).waitFor(); assert.equal(await draftForm.getByLabel('营业日期').inputValue(), unknownDay)
  const countBeforeReadback = operations.filter(item => item.method === 'POST' && item.path === lost.path).length
  const readbackResponse = view.page.waitForResponse(response => {
    const url = new URL(response.url())
    return response.request().method() === 'GET' && url.pathname === lost.path && url.searchParams.get('dateFrom') === unknownDay && url.searchParams.get('dateTo') === unknownDay
  })
  await draftForm.getByRole('button', { name: '读回核对', exact: true }).click()
  const originalRows = await (await readbackResponse).json(), original = originalRows.records.find(row => row.id === lost.body.id)
  assert(original); assert.equal(original.organizationId, actor.organizationId); assert.equal(original.storeId, store.id); assert.equal(original.ownerUserId, actor.userId); assert.equal(original.businessDate, unknownDay); assert.equal(original.amount, '7.89')
  await draftForm.getByRole('button', { name: '已核对，允许新操作', exact: true }).click()
  assert.equal(operations.filter(item => item.method === 'POST' && item.path === lost.path).length, countBeforeReadback)
  faults.push({ ...lost, body: undefined }); check('real-write-response-loss-readback', { reportId: lost.body.id, originalDate: unknownDay, actualCommitted: true, readback: true, automaticRetries: 0 })

  const writeOnly = await open(services.actors.createOnly, '/micro/stores/store-create'); await active(writeOnly)
  const writeForm = form(writeOnly, 'store-create'); await writeForm.getByLabel('门店名称').fill('Phase27 write-only committed')
  const lostWrite = await dropCommitted(writeOnly, API, 'POST', () => writeForm.getByRole('button', { name: '创建门店', exact: true }).click())
  assert.equal((await services.businessDb.query('SELECT id FROM stores WHERE id=$1', [lostWrite.body.id])).rows.length, 1); lostWrite.actualUpstreamCommitted = true
  await writeForm.getByText(/本次写入结果未知/).waitFor(); await writeForm.getByText(/无.*读取权限|没有.*读取权限|不具备.*读取权限/).waitFor()
  assert.equal(await writeForm.getByLabel('门店名称').inputValue(), 'Phase27 write-only committed')
  faults.push({ ...lostWrite, body: undefined }); check('write-only-unknown-no-retry', { storeId: lostWrite.body.id, committed: true, readPermission: false, automaticRetries: 0 })

  const workActor = await services.authorize(actor, 'A', 'work-orders')
  const work = await pageIn(view.context, 'work-orders', '/micro/work-orders/ticket-create'); await active(work)
  await work.frame.getByTestId('ticket-title').fill('Phase27 双消费者实际工单')
  await work.frame.getByTestId('ticket-assignee').locator(`option[value="${peer.userId}"]`).waitFor({ state: 'attached' })
  await work.frame.getByTestId('ticket-assignee').selectOption(peer.userId)
  const createdWork = work.page.waitForResponse(response => new URL(response.url()).pathname === '/work-orders-api/tickets' && response.request().method() === 'POST')
  await work.frame.getByTestId('save-ticket').click(); const workResponse = await createdWork; assert.equal(workResponse.status(), 201); const ticket = await workResponse.json()
  await navigate(work, '/micro/work-orders/tickets/' + ticket.id); await work.frame.getByTestId('ticket-detail').waitFor()
  const memberStore = (await business(actor, '/stores-api/members?resource=store&action=update&organizationId=' + actor.organizationId)).data
  const memberWork = (await business(workActor, '/work-orders-api/assignees?action=update&organizationId=' + actor.organizationId)).data
  assert(memberStore.records.some(row => row.id === peer.userId)); assert(memberWork.records.some(row => row.userId === peer.userId))
  const cross = await business(workActor, API, {}, 401); await business(actor, '/work-orders-api/tickets', {}, 401)
  const undeclared = await business(actor, '/stores-api/members?resource=store&action=read', {}, 400)
  await services.platformApi('/api/apps/stores/members?resource=store&action=read&organizationId=' + actor.organizationId, { token: actor.token }, 400)
  await services.platformApi('/api/apps/stores/members?resource=store&action=create&organizationId=' + actor.organizationId, { token: workActor.token }, 403)
  await business(services.actors.readOnly, API, { method: 'POST', body: { name: '拒绝缺功能', businessStatus: 'OPEN', businessContact: '' } }, 403)
  check('cross-app-bearer-permission-grant-denials', { crossApp: cross.status, undeclaredMemberPurpose: undeclared.status, actualPlatformMemberPurpose: 400, actualPlatformCrossApp: 403, noCreatePermission: 403 })

  // A genuine legal renewal is awaited while the same ordinary user retains an unsent draft.
  await navigate(view, '/micro/stores/daily-report-create'); await form(view, 'report-create').getByLabel('营业金额').fill('11.22')
  const renewalFormId = await form(view, 'report-create').getAttribute('data-form-id')
  const initialIssueCount = view.issuances.filter(item => item.status === 201).length

  const revoked = await open(await services.authorize(services.actors.peer), '/micro/stores'); await active(revoked)
  await revoked.frame.locator(`[data-row-id="${store.id}"]`).waitFor()
  await services.setGrants(services.actors.peer, { remove: ['store:read'] })
  await clicked(revoked, 'store-query', '查询门店', API, 'GET', 403)
  await wait(async () => await revoked.frame.locator('[data-row-id]').count() === 0, '403 clears old server rows')
  await revoked.page.getByRole('button', { name: /切换到.*模式/ }).click()
  assert.equal(await revoked.frame.locator('[data-row-id]').count(), 0)
  check('revocation403-freezes-clears', { actualRustStatus: 403, oldRowsVisible: false, themeDoesNotRestore: true })
  const unauthorizedActor = await services.authorize(services.actors.readOnly), unauthorized = await open(unauthorizedActor, '/micro/stores'); await active(unauthorized)
  await unauthorized.frame.locator(`[data-row-id="${store.id}"]`).waitFor()
  await services.platformApi('/api/auth/logout', { token: unauthorizedActor.session, method: 'POST' })
  await clicked(unauthorized, 'store-query', '查询门店', API, 'GET', 401)
  await wait(async () => await unauthorized.frame.locator('[data-row-id]').count() === 0, '401 immediately clears old server views')
  check('unauthorized401-clears', { actualRustStatus: 401, oldRowsVisible: false })

  const routeActor = await services.authorize(services.actors.child), routed = await open(routeActor, '/micro/stores/reports'); await active(routed)
  let releaseDelay, delayed = false
  const gate = new Promise(resolve => { releaseDelay = resolve })
  const delayHandler = async route => { if (route.request().method() === 'GET' && new URL(route.request().url()).pathname === '/stores-api/daily-reports/summary' && !delayed) { delayed = true; const actual = await route.fetch(); await gate; try { await route.fulfill({ response: actual }) } catch {} } else await route.continue() }
  await routed.context.route('**/*', delayHandler)
  await form(routed, 'summary').getByLabel('门店ID').fill(store.id); await form(routed, 'summary').getByLabel('开始日期').fill(yesterday); await form(routed, 'summary').getByLabel('结束日期').fill(today)
  await form(routed, 'summary').getByRole('button', { name: '查询汇总', exact: true }).click(); await wait(() => delayed, 'Actual response delay starts')
  await navigate(routed, '/micro/stores/staff'); releaseDelay(); await routed.context.unroute('**/*', delayHandler)
  assert.equal(await routed.frame.getByTestId('server-summary').count(), 0)
  await navigate(routed, '/micro/stores/does-not-exist'); await routed.page.getByText(/无访问权限|页面不存在|未找到/).first().waitFor()
  check('route-cancellation-and-not-found', { actualResponseDelayed: true, departedSummaryVisible: false, unknownRouteDenied: true })

  await wait(async () => view.issuances.filter(item => item.status === 201).length > initialIssueCount && await form(view, 'report-create').getAttribute('data-form-id') !== renewalFormId,
    'Actual 300-second confidential-client application token renewal', Math.max(30000, 330000 - (Date.now() - started)))
  assert.equal(await form(view, 'report-create').getByLabel('营业金额').inputValue(), '11.22')
  check('normal-renewal-keeps-legal-input', { actualAuthorizationReplacement: true, initialIssueCount, finalIssueCount: view.issuances.filter(item => item.status === 201).length,
    actualIssueStatus: 201, sameIdentityDraft: '11.22', elapsedMs: Date.now() - started })
  await screenshot(view, 'phase27-real-form-dark')

  await form(view, 'report-create').evaluate(element => {
    const key = Object.keys(element).find(key => key.startsWith('__reactProps$'))
    window.__phase27OldSubmit = element[key].onSubmit
  })
  const summaryPage = await pageIn(view.context, 'stores', '/micro/stores/reports'); await active(summaryPage)
  await form(summaryPage, 'summary').getByLabel('门店ID').fill(store.id)
  await form(summaryPage, 'summary').getByLabel('开始日期').fill(unknownDay); await form(summaryPage, 'summary').getByLabel('结束日期').fill(today)
  await clicked(summaryPage, 'summary', '查询汇总', '/stores-api/daily-reports/summary', 'GET', 200)
  await summaryPage.frame.getByTestId('server-summary').waitFor()
  const directoryPage = await pageIn(view.context, 'stores', '/micro/stores/stores/' + store.id); await active(directoryPage)
  await form(directoryPage, 'store-edit').getByLabel('成员搜索').fill('peer')
  await form(directoryPage, 'store-edit').getByRole('button', { name: '搜索成员', exact: true }).click()
  await form(directoryPage, 'store-edit').getByRole('button', { name: '选择 Phase27 peer', exact: true }).waitFor()
  const orgB = await services.authorize(actor, 'B'), storeB = (await business(orgB, API, { method: 'POST', body: { name: 'Phase27 B门店', businessStatus: 'OPEN', businessContact: 'B组织独立' } }, 201)).data
  await view.page.getByRole('button', { name: '切换组织上下文', exact: true }).click()
  await view.page.getByText('Phase27 organization B', { exact: true }).click()
  await wait(async () => {
    try { return await form(view, 'report-create').getByLabel('营业金额').inputValue() !== '11.22' } catch { return false }
  }, 'Both actual pages observe shared organization B')
  await wait(async () => !await work.frame.locator(`[data-ticket-id="${ticket.id}"]`).count(), 'Old organization A work-order data disappears')
  await work.frame.locator(`[data-testid="work-orders-app"][data-org-id="${orgB.organizationId}"]`).waitFor()
  await wait(async () => !await summaryPage.frame.getByTestId('server-summary').count(), 'Old organization A server summary disappears')
  await wait(async () => !await directoryPage.frame.getByRole('button', { name: '选择 Phase27 peer', exact: true }).count(), 'Old organization A directory disappears')
  const beforeStale = operations.filter(item => ['POST', 'PATCH', 'DELETE'].includes(item.method)).length
  let oldHandlerOutcome = 'iframe-replaced'
  try { oldHandlerOutcome = await view.frame.locator('body').evaluate(() => {
    if (!window.__phase27OldSubmit) return 'iframe-replaced'
    window.__phase27OldSubmit({ preventDefault() {} }); return 'old-handler-invoked'
  }) } catch {}
  await delay(100)
  assert.equal(operations.filter(item => ['POST', 'PATCH', 'DELETE'].includes(item.method)).length, beforeStale)
  check('stale-form-handler-cannot-cross-org', { oldHandlerOutcome, crossOrganizationWrites: 0, oldAmount: '11.22' })
  await navigate(view, '/micro/stores'); await view.frame.locator(`[data-row-id="${storeB.id}"]`).waitFor()
  assert.equal(await view.frame.locator(`[data-row-id="${store.id}"]`).count(), 0)
  await navigate(work, '/micro/work-orders'); await work.frame.getByTestId('ticket-list').waitFor()
  assert.equal(await work.frame.locator(`[data-ticket-id="${ticket.id}"]`).count(), 0)
  check('dual-host-shared-session-org-switch', { actualPages: 4, actualApplications: 2, organizationA: actor.organizationId, organizationB: orgB.organizationId,
    oldStoresVisible: false, oldTicketVisible: false, oldSummaryVisible: false, oldDirectoryVisible: false, authoritativeSnapshotsExistedBeforeSwitch: true })
  await view.page.reload({ waitUntil: 'domcontentloaded' }); await active(view); await view.frame.locator(`[data-row-id="${storeB.id}"]`).waitFor()
  await work.page.reload({ waitUntil: 'domcontentloaded' }); await active(work)
  await view.page.getByRole('button', { name: '用户菜单', exact: true }).click(); await view.page.getByText('退出登录', { exact: true }).click()
  await view.page.waitForURL(url => url.pathname === '/login'); await work.page.waitForURL(url => url.pathname === '/login')
  await summaryPage.page.waitForURL(url => url.pathname === '/login'); await directoryPage.page.waitForURL(url => url.pathname === '/login')
  check('dual-host-refresh-logout-fences', { refreshedPages: 2, logoutPages: 4, oldDirectoryVisible: false, oldSummaryVisible: false })

  const httpMembers = services.memberHttpObservations()
  const memberProof = { stores: { requests: httpMembers.filter(item => item.app === 'stores'), records: memberStore.records.map(row => ({ id: row.id, organizationId: row.organizationId })), idField: 'id', actualPlatformHttp: true },
    'work-orders': { requests: httpMembers.filter(item => item.app === 'work-orders'), records: memberWork.records.map(row => ({ userId: row.userId, organizationId: row.organizationId })), idField: 'userId', actualPlatformHttp: true }, crossAppStatus: 401, platformCrossAppStatus: 403, undeclaredPurposeStatus: 400 }
  assert(memberProof.stores.requests.length && memberProof['work-orders'].requests.length)
  evidence.members = memberProof
  check('dual-actual-controlled-member-http', { stores: memberProof.stores.requests.length, workOrders: memberProof['work-orders'].requests.length, platformProtocol: '/api/apps/{app}/members', storesId: 'id', workOrdersId: 'userId' })
  const preflight = await services.request(services.origin, API, { method: 'OPTIONS', headers: { Origin: 'null', 'Access-Control-Request-Method': 'GET', 'Access-Control-Request-Headers': 'authorization,x-request-id' } })
  assert.equal(preflight.status, 204); assert.equal(preflight.headers['access-control-allow-origin'], 'null'); assert(!preflight.headers['access-control-allow-credentials'])
  const workPreflight = await services.request(services.origin, '/work-orders-api/tickets', { method: 'OPTIONS', headers: { Origin: 'null', 'Access-Control-Request-Method': 'GET', 'Access-Control-Request-Headers': 'authorization,x-request-id' } })
  assert.equal(workPreflight.status, 204); assert.equal(workPreflight.headers['access-control-allow-origin'], 'null'); assert(!workPreflight.headers['access-control-allow-credentials'])
  check('opaque-origin-null-rust-no-cookie', { actualPreflight: 204, workOrdersPreflight: 204, effectiveOrigin: 'null', credentials: 'omit' })
  assert.deepEqual(errors, [], 'Actual production browser cannot have uncaught JavaScript errors')
  await wait(() => [...native.values()].every(record => record.nativeOutcome), 'Every observed native business fetch settles before context closure', 20000)
  await Promise.allSettled([...headerTasks])
} catch (error) { failure = error; console.error(services ? services.redact(error.stack) : error.stack) }
finally {
  for (const context of contexts) try { await context.close() } catch {}
  if (browser) try { await browser.close(); browserClosed = true } catch {}
  await Promise.allSettled([...headerTasks])
  if (services) try { cleanup = await services.close({ browserClosed }) } catch (error) { cleanup = error.phase27Cleanup; failure ||= error; console.error(error.message) }
  else cleanup = failure?.phase27StartupCleanup
  if (!cleanup && failure) console.log('PHASE27_BROWSER_CLEANUP ' + JSON.stringify({ startupFailed: true }))
}
if (!failure) {
  for (const fault of faults) {
    const index = operations.findIndex(item => item.requestId === fault.requestId); assert(index >= 0)
    fault.automaticRetryCount = operations.slice(index + 1).filter(item => item.method === fault.method && item.path === fault.path).length
    assert.equal(fault.automaticRetryCount, 0, 'A frozen unknown write must not be automatically resent')
  }
  const records = [...native.values()].map(record => {
    assert(record.started && record.nativeOutcome, 'Every observed native fetch must settle')
    const wire = operations.find(item => item.requestId === record.requestId) || null
    return { requestId: record.requestId, method: record.method, path: record.path, credentials: record.credentials, cookiePresent: record.cookiePresent,
      bearerPresent: record.bearerPresent, idempotencyKeyPresent: !!record.idempotencyKeyPresent, nativeOutcome: record.nativeOutcome, nativeErrorName: record.nativeErrorName || null, signalAborted: !!record.signalAborted, wire }
  })
  const ordered = checksRequired.map(id => { const actual = checks.find(item => item.id === id); assert(actual, 'Missing mandatory browser check: ' + id); return actual })
  const result = { schemaVersion: 1, product: browserName, browserVersion: browser.version(), playwright: playwrightVersion, sourceSha256: sha(readFileSync(fileURLToPath(import.meta.url))),
    passed: ordered.length, failed: 0, skipped: 0, checks: ordered, source: services.source, evidence, members: evidence.members,
    transport: { origin: 'null', cookiePresent: false, credentials: 'omit', rustPreflightStatus: 204, credentialsAllowed: false, nativeAttemptCount: records.length, records, faults } }
  console.log('PHASE27_BROWSER_RESULT ' + JSON.stringify(result))
} else process.exitCode = 1
