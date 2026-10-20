/** Real Vue/registered bridge coverage; the business HTTP service is an isolated fixture. */
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'

const APP = 'work-orders'
const API = '/work-orders-api'
const ENTRY = '/__phase8-work-orders/'
const actions = ['read', 'create', 'update', 'delete', 'export']
const permissions = actions.map((action) => 'ticket:' + action)
const grants = actions.map((action) => ({ resource: 'ticket', action, scope: 'ORG',
  subjectUserId: 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', tenantId: 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', organizationIds: ['cccccccc-cccc-4ccc-8ccc-cccccccccccc'] }))
const application = { id: 'application-work-orders', code: APP, name: '真实工单更新验收', icon: 'Boxes',
  basePath: '/micro/' + APP, runtimeMode: 'IFRAME', entryType: 'HTML', entryUrl: ENTRY,
  deploymentVersion: 'phase8-fixture', entryVersionId: 'release-work-orders-phase8-fixture' }
const menus = [{ id: 'work-orders-home', parentId: null, type: 'MENU', title: '工单列表',
  path: '/micro/' + APP, permission: 'ticket:read', sortOrder: 1, visible: true, children: [] },
{ id: 'work-orders-create', parentId: null, type: 'MENU', title: '新建工单',
  path: '/micro/' + APP + '/ticket-create', permission: 'ticket:create', sortOrder: 2, visible: true, children: [] },
{ id: 'work-orders-detail', parentId: null, type: 'MENU', title: '工单详情',
  path: '/micro/' + APP + '/tickets/:id', permission: 'ticket:read', sortOrder: 3, visible: false, children: [] }]
const deferred = () => { let resolve; const promise = new Promise((done) => { resolve = done }); return { promise, resolve } }
const ticket = (id, title, version = 1) => ({ id, title, version, number: 'WO-' + id,
  description: '真实 Vue 工单夹具', status: 'OPEN', priority: 'NORMAL', tenantId: 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb',
  organizationId: 'cccccccc-cccc-4ccc-8ccc-cccccccccccc', organizationName: '组织 A', ownerUserId: 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',
  ownerDisplayName: '浏览器验收用户', assigneeUserId: null, assigneeDisplayName: null,
  createdAt: '2026-10-15T00:00:00.000Z', updatedAt: '2026-10-15T00:00:00.000Z' })
const routePath = (id) => '/micro/' + APP + (id ? '/tickets/' + id : '')
const canonicalIdentity = value => JSON.parse(JSON.stringify(value).replaceAll('user-browser-test', 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa')
  .replaceAll('tenant-A', 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb').replaceAll('org-A', 'cccccccc-cccc-4ccc-8ccc-cccccccccccc'))

// This observes the actual signal delivered by WorkOrdersClient -> registered SDK
// -> native fetch. One explicitly armed call can ignore native transport abort to
// exercise a real late HTTP response; no SDK result or application state is faked.
function installFetchObservation() {
  if (window.top === window) return
  const nativeFetch = window.fetch.bind(window)
  const observation = window.__phase8WorkOrdersFetch = { events: [], detachNext: false }
  window.fetch = function (input, options = {}) {
    const url = new URL(typeof input === 'string' ? input : input.url, location.href)
    if (!url.pathname.startsWith('/work-orders-api/')) return nativeFetch(input, options)
    const signal = options.signal
    const detached = observation.detachNext
    observation.detachNext = false
    const event = { id: observation.events.length + 1, method: options.method || 'GET', path: url.pathname,
      aborted: Boolean(signal?.aborted), nativeSignalDetached: detached, completed: false, failed: false }
    const record = () => { void window.__phase8RecordWorkOrdersFetch({ ...event }).catch(() => {}) }
    observation.events.push(event)
    record()
    signal?.addEventListener('abort', () => { event.aborted = true; record() }, { once: true })
    return nativeFetch(input, detached ? { ...options, signal: undefined } : options).then((response) => {
      event.completed = true; event.completedAfterAbort = event.aborted; event.status = response.status
      record()
      return response
    }, (error) => { event.failed = true; event.errorName = error.name; record(); throw error })
  }
}

async function navigate(page, pathname) {
  await page.evaluate((next) => { history.pushState(null, '', next); dispatchEvent(new PopStateEvent('popstate')) }, pathname)
  await page.waitForURL((url) => url.pathname === pathname)
}
const frameFor = (page) => page.frameLocator('.micro-mount-zone iframe')
const trace = (frame) => frame.locator('body').evaluate(() => window.__phase8WorkOrdersFetch.events)
const lastFetch = async (frame, method, pathname) => (await trace(frame)).findLast((entry) => entry.method === method && entry.path === pathname)
const readCount = (api) => api.requests.filter((entry) => entry.method === 'GET').length
const writeCount = (api) => api.requests.filter((entry) => ['POST', 'PATCH', 'DELETE'].includes(entry.method)).length
const emit = (scenario, data) => console.log('PHASE8_WORK_ORDERS_TRACE ' + JSON.stringify({ scenario, ...data }))

function createApiFixture() {
  return { rows: [ticket('11111111-1111-4111-8111-111111111111', '原始工单一'), ticket('22222222-2222-4222-8222-222222222222', '原始工单二')],
    requests: [], staticRequests: [], fetchEvents: [], pending: [], holdNext: null, hold: null }
}

async function setupFixture({ context, state, leases, origin }, api, output) {
  await context.exposeBinding('__phase8RecordWorkOrdersFetch', (_source, event) => { api.fetchEvents.push(event) })
  await context.addInitScript(installFetchObservation)
  await context.route('**/__phase8-work-orders/**', async (route) => {
    const pathname = new URL(route.request().url()).pathname
    const relative = pathname.slice(ENTRY.length) || 'index.html'
    api.staticRequests.push(relative)
    const target = path.resolve(output, relative)
    assert.ok(target.startsWith(path.resolve(output) + path.sep), 'Static fixture must remain in its owned build output.')
    if (!existsSync(target)) return route.fulfill({ status: 404, body: 'Unknown work orders fixture resource' })
    const contentType = target.endsWith('.html') ? 'text/html' : target.endsWith('.css') ? 'text/css' : 'text/javascript'
    await route.fulfill({ status: 200, headers: { 'Access-Control-Allow-Origin': '*' }, contentType, body: readFileSync(target) })
  })
  await context.route('**/work-orders-api/**', async (route) => {
    const request = route.request(), url = new URL(request.url()), headers = await request.allHeaders()
    assert.equal(url.origin, origin)
    const cors = { 'Access-Control-Allow-Origin': 'null', 'Access-Control-Allow-Methods': 'GET,POST,PATCH,DELETE,OPTIONS',
      'Access-Control-Allow-Headers': 'Authorization,Content-Type,Idempotency-Key,X-Request-Id', 'Access-Control-Expose-Headers': 'X-Request-Id' }
    if (request.method() === 'OPTIONS') return route.fulfill({ status: 204, headers: cors })
    const lease = leases.get(headers.authorization?.replace(/^Bearer /, ''))
    assert.equal(lease?.appCode, APP, 'Actual business requests must carry an issued application Bearer, never the platform session.')
    assert.equal(headers.origin, 'null', 'Actual business requests originate in the opaque iframe.')
    assert.match(headers['x-request-id'], /^[0-9a-f-]{36}$/i, 'The registered SDK must supply a request UUID.')
    const method = request.method(), pathname = url.pathname, body = request.postData() ? request.postDataJSON() : undefined
    const entry = { ordinal: api.requests.length + 1, method, path: pathname, applicationBearerVerified: true,
      opaqueOriginVerified: true, requestIdPresent: true, ...(body ? { body } : {}) }
    api.requests.push(entry)
    const send = async (value, status = 200) => {
      try { await route.fulfill({ status, headers: { ...cors, 'X-Request-Id': headers['x-request-id'] },
        contentType: 'application/json', body: JSON.stringify(value) }) }
      catch (error) { if (!/closed|disposed|Invalid Interception|Target|cancelled/i.test(error.message)) throw error }
    }
    let result, status = 200
    if (pathname === API + '/assignees' && method === 'GET') result = { records: [], total: 0, page: 1, size: 50 }
    else if (pathname === API + '/tickets' && method === 'GET') result = { records: api.rows, total: api.rows.length, page: 1, size: 20 }
    else if (pathname === API + '/tickets' && method === 'POST') { result = ticket('33333333-3333-4333-8333-333333333333', body.title); status = 201 }
    else {
      const id = pathname.slice((API + '/tickets/').length), current = api.rows.find((row) => row.id === id)
      assert.ok(current, 'The business fixture must receive a known ticket path.')
      if (method === 'GET') result = current
      else if (method === 'PATCH') result = { ...current, ...body, version: current.version + 1 }
      else throw new Error('Unexpected work orders business operation: ' + method + ' ' + pathname)
    }
    if (api.holdNext?.method === method && api.holdNext.path === pathname) {
      const hold = { ...deferred(), request: entry, result, status }
      api.pending.push(hold); api.hold = hold; api.holdNext = null
      await hold.promise
      // An intentionally stale response is returned verbatim. It is never
      // installed into the fixture's latest server state after authorization changes.
      if (!hold.suppressCommit && ['POST', 'PATCH'].includes(method)) {
        api.rows = [...api.rows.filter((row) => row.id !== result.id), result]
      }
    } else if (['POST', 'PATCH'].includes(method)) {
      api.rows = [...api.rows.filter((row) => row.id !== result.id), result]
    }
    await send(result, status)
  })
  state.phase8WorkOrdersApi = api
}

export async function verifyPhase8WorkOrders({ withFixture, pause, waitState, refreshContext, workspace, origin }) {
  const expectedOrigin = typeof origin === 'function' ? origin() : origin
  const sourceRoot = process.env.MICRO_HOST_BASELINE_ROOT || workspace
  const temporary = mkdtempSync(path.join(os.tmpdir(), 'saas-phase8-work-orders-browser-'))
  const output = path.join(temporary, 'app')
  const build = spawnSync(process.execPath, [path.join(sourceRoot, 'business-apps/work-orders/frontend/scripts/build.mjs'),
    '--out-dir', output, '--version', 'phase8-fixture', '--allowed-host-origin', expectedOrigin],
  { cwd: workspace, encoding: 'utf8', env: { ...process.env, NODE_PATH: [
    path.join(workspace, 'business-apps/work-orders/frontend/node_modules'), path.join(workspace, 'frontend/node_modules'),
    process.env.NODE_PATH].filter(Boolean).join(path.delimiter) }, timeout: 60_000 })
  if (build.status !== 0) {
    assert.equal(path.dirname(path.resolve(temporary)), path.resolve(os.tmpdir()))
    assert.ok(path.basename(temporary).startsWith('saas-phase8-work-orders-browser-'))
    rmSync(temporary, { recursive: true, force: true })
  }
  assert.equal(build.status, 0, 'Actual standalone Vue build failed: ' + build.stdout + build.stderr)
  emit('actual-build-inputs', { baselineOverride: Boolean(process.env.MICRO_HOST_BASELINE_ROOT), files:
    ['src/app.mjs', 'src/client.mjs', 'src/domain.mjs', 'vendor/iframe-bridge.js', 'vendor/micro-sdk.mjs'].map((relative) => ({
      path: 'business-apps/work-orders/frontend/' + relative,
      sha256: createHash('sha256').update(readFileSync(path.join(sourceRoot, 'business-apps/work-orders/frontend', relative))).digest('hex') })) })
  const run = async (startId, test, options = {}) => {
    const api = createApiFixture()
    try {
      await withFixture({ applications: [application], startPath: routePath(startId), exposeRuntime: true,
        tokenExpiresIn: 600, contextTtlMs: 600_000, contextPermissions: permissions, contextDataGrants: grants,
        profileResponseTransform: canonicalIdentity, contextResponseTransform: canonicalIdentity,
        contextMenus: menus, ...options, setup: (fixture) => setupFixture(fixture, api, output) }, async (fixture) => {
        const frame = frameFor(fixture.page)
        try { await frame.locator('[data-testid="work-orders-app"][data-context-active="true"]').waitFor() }
        catch (error) { console.error('PHASE8_WORK_ORDERS_BOOT_DIAGNOSTIC ' + JSON.stringify({ staticRequests: api.staticRequests,
          iframe: await frame.locator('body').evaluate(() => ({ body: document.body.textContent.slice(0, 2000),
            configPresent: Boolean(window.__WORK_ORDERS_CONFIG__), bridgePresent: Boolean(window.MicroIframe?.connect),
            modules: [...document.querySelectorAll('script[type="module"]')].map(script => script.src) })).catch(() => null) })); throw error }
        await fixture.page.getByTestId('micro-status-bar').getByText('运行中', { exact: true }).waitFor()
        if (startId) await frame.locator('[data-testid="ticket-detail"] [data-ticket-id="' + startId + '"]').waitFor()
        else await frame.getByTestId('ticket-row').first().waitFor()
        assert.equal(await fixture.page.locator('.micro-mount-zone iframe').getAttribute('sandbox'), 'allow-scripts allow-forms')
        await test({ ...fixture, frame, api })
      })
    } finally { for (const held of api.pending) held.resolve() }
  }
  const hold = async (api, method, pathname, action) => {
    api.hold = null; api.holdNext = { method, path: pathname }
    await action()
    await waitState(() => api.hold, 'Actual ' + method + ' HTTP request must reach the deferred business fixture.')
    return api.hold
  }
  try {
    await run(null, async ({ page, frame, api }) => {
      await frame.getByTestId('create-ticket').click()
      await page.waitForURL((url) => url.pathname === '/micro/' + APP + '/ticket-create')
      await frame.getByTestId('ticket-title').fill('主题切换中的新建提交')
      await waitState(() => api.requests.some((entry) => entry.path === API + '/assignees'), 'Create assignees must load before the theme comparison.')
      const create = await hold(api, 'POST', API + '/tickets', () => frame.getByTestId('save-ticket').click())
      const beforeCreateReads = readCount(api)
      await page.getByRole('button', { name: '切换到深色模式', exact: true }).click()
      await frame.locator('html[data-theme="dark"]').waitFor()
      const pendingCreate = await lastFetch(frame, 'POST', API + '/tickets')
      assert.equal(pendingCreate.aborted, false, 'Theme updates must retain the actual pending create request signal.')
      assert.equal(readCount(api), beforeCreateReads, 'Pure theme updates must not refetch work order lists/details/assignees.')
      assert.equal(await frame.getByTestId('save-ticket').isDisabled(), true)
      create.resolve()
      await frame.getByTestId('ticket-detail').getByRole('heading', { name: '主题切换中的新建提交', exact: true }).waitFor()
      await frame.getByTestId('edit-ticket').click()
      await frame.getByTestId('ticket-title').fill('主题切换中的编辑提交')
      const edit = await hold(api, 'PATCH', API + '/tickets/33333333-3333-4333-8333-333333333333', () => frame.getByTestId('save-ticket').click())
      const beforeEditReads = readCount(api)
      await page.getByRole('button', { name: '切换到浅色模式', exact: true }).click()
      await frame.locator('html[data-theme="light"]').waitFor()
      const pendingEdit = await lastFetch(frame, 'PATCH', API + '/tickets/33333333-3333-4333-8333-333333333333')
      assert.equal(pendingEdit.aborted, false, 'Theme updates must retain the actual pending edit request signal.')
      assert.equal(readCount(api), beforeEditReads)
      edit.resolve()
      await frame.getByTestId('ticket-detail').getByRole('heading', { name: '主题切换中的编辑提交', exact: true }).waitFor()
      emit('theme-preserves-create-and-update', { create: pendingCreate, update: pendingEdit, readCount: beforeEditReads })
      console.log('PASS actual Vue work orders: pure theme retains pending create/update signals, busy state and cached reads')
    })

    await run('11111111-1111-4111-8111-111111111111', async ({ page, frame, api }) => {
      await frame.getByTestId('edit-ticket').click()
      await frame.getByTestId('ticket-title').fill('路由切换中的编辑提交')
      const write = await hold(api, 'PATCH', API + '/tickets/11111111-1111-4111-8111-111111111111', () => frame.getByTestId('save-ticket').click())
      const read = await hold(api, 'GET', API + '/tickets/22222222-2222-4222-8222-222222222222', () => navigate(page, routePath('22222222-2222-4222-8222-222222222222')))
      await navigate(page, routePath('11111111-1111-4111-8111-111111111111'))
      await frame.locator('[data-testid="ticket-detail"] [data-ticket-id="11111111-1111-4111-8111-111111111111"]').waitFor()
      const pendingWrite = await lastFetch(frame, 'PATCH', API + '/tickets/11111111-1111-4111-8111-111111111111')
      const retiredRead = await lastFetch(frame, 'GET', API + '/tickets/22222222-2222-4222-8222-222222222222')
      assert.equal(pendingWrite.aborted, false, 'Route updates must not abort the pending business write.')
      assert.equal(retiredRead.aborted, true, 'The read for the departed route must be cancelled independently.')
      assert.equal(api.requests.filter((entry) => entry.method === 'GET' && entry.path === API + '/tickets').length, 0,
        'Detail-to-detail navigation must not trigger an unrelated list request.')
      read.resolve(); write.resolve()
      await frame.getByTestId('ticket-detail').getByRole('heading', { name: '路由切换中的编辑提交', exact: true }).waitFor()
      emit('route-cancels-only-related-read', { pendingWrite, retiredRead })
      console.log('PASS actual Vue work orders: route cancels its retired detail read while the pending write completes')
    })

    await run('11111111-1111-4111-8111-111111111111', async ({ page, state, frame, api }) => {
      await frame.getByTestId('edit-ticket').click()
      await frame.getByTestId('ticket-title').fill('禁止回写的旧续期响应')
      await frame.locator('body').evaluate(() => { window.__phase8WorkOrdersFetch.detachNext = true })
      const write = await hold(api, 'PATCH', API + '/tickets/11111111-1111-4111-8111-111111111111', () => frame.getByTestId('save-ticket').click())
      write.suppressCommit = true
      const issues = state.issueCount
      await waitState(() => state.issueCount > issues, 'Credential renewal must issue a real replacement application token.')
      await frame.locator('[data-testid="work-orders-app"][data-context-active="true"]').waitFor()
      await frame.locator('body').evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))))
      const cancelled = await lastFetch(frame, 'PATCH', API + '/tickets/11111111-1111-4111-8111-111111111111')
      assert.equal(cancelled.aborted, true, 'Token renewal must abort the old request signal even when the injected native transport ignores it.')
      assert.equal(cancelled.nativeSignalDetached, true)
      write.resolve()
      await frame.locator('body').evaluate(() => new Promise((resolve, reject) => {
        const deadline = performance.now() + 5000
        function check() {
          const event = window.__phase8WorkOrdersFetch.events.findLast((entry) => entry.method === 'PATCH')
          if (event?.completed) resolve()
          else if (performance.now() > deadline) reject(new Error('The deliberately late native HTTP response never completed.'))
          else requestAnimationFrame(check)
        }
        check()
      }))
      await pause(100)
      assert.equal(await frame.getByTestId('ticket-detail').getByRole('heading', { name: '禁止回写的旧续期响应', exact: true }).count(), 0)
      assert.equal(await frame.getByText('工单更改已保存', { exact: true }).count(), 0)
      assert.equal(await frame.getByTestId('ticket-title').inputValue(), '禁止回写的旧续期响应', 'Same identity draft may survive renewal without treating the old write as committed.')
      const late = await lastFetch(frame, 'PATCH', API + '/tickets/11111111-1111-4111-8111-111111111111')
      assert.equal(late.completedAfterAbort, true)
      emit('renewal-rejects-real-late-http-response', { late, tokenIssueDelta: state.issueCount - issues })
      console.log('PASS actual Vue work orders: credential renewal aborts the old signal and rejects a real late write response without success UI')
    }, { tokenExpiresIn: 4 })

    await run('11111111-1111-4111-8111-111111111111', async ({ page, state, frame, api }) => {
      await frame.getByTestId('change-ticket-status').evaluate((button) => { window.__phase8RetiredWorkOrderAction = button })
      state.contextDataGrants = []; state.version = 'grant-revoked'
      await refreshContext(page)
      await frame.locator('[data-testid="work-orders-app"][data-context-active="true"]').waitFor()
      await frame.locator('body').evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))))
      assert.equal(await frame.getByTestId('edit-ticket').count(), 0)
      assert.equal(await frame.locator('[data-ticket-id="11111111-1111-4111-8111-111111111111"]').count(), 0, 'Revoked grants must clear the former server record.')
      const beforeStaleClick = writeCount(api)
      await frame.locator('body').evaluate(() => window.__phase8RetiredWorkOrderAction.click())
      await pause(50)
      assert.equal(writeCount(api), beforeStaleClick, 'Even a retained old DOM handler cannot operate on a revoked ticket.')
      state.contextDataGrants = grants; state.version = 'grant-restored'
      await refreshContext(page)
      await frame.getByTestId('change-ticket-status').waitFor()
      const write = await hold(api, 'PATCH', API + '/tickets/11111111-1111-4111-8111-111111111111', () => frame.getByTestId('change-ticket-status').click())
      state.failAt = 'context'; state.failStatus = 403
      await refreshContext(page)
      await page.waitForURL('**/dashboard')
      await page.getByText('当前应用授权已失效，已退出应用', { exact: true }).waitFor()
      await waitState(() => api.fetchEvents.some((entry) => entry.method === 'PATCH' && entry.aborted),
        'Definitive membership refusal must abort the actual pending application signal before the frame retires.')
      const refused = api.fetchEvents.findLast((entry) => entry.method === 'PATCH' && entry.aborted)
      assert.equal(await page.locator('.micro-mount-zone iframe').count(), 0, 'Definitive membership refusal removes the previous application frame.')
      const afterMemberRevoke = writeCount(api)
      write.resolve(); await pause(100)
      assert.equal(writeCount(api), afterMemberRevoke)
      assert.equal(new URL(page.url()).pathname, '/dashboard')
      emit('grant-and-membership-revocation', { retiredGrantHandlerWriteDelta: 0, memberWriteCount: afterMemberRevoke,
        definitiveContextStatus: 403, retiredFrameRemoved: true, ...(refused ? { cancelledRequest: refused } : {}) })
      console.log('PASS actual Vue work orders: revoked grants clear old records/handlers, definitive membership refusal cancels and removes the old frame')
    })
  } finally {
    if (existsSync(path.join(output, 'build-info.json'))) assert.equal(JSON.parse(readFileSync(path.join(output, 'build-info.json'), 'utf8')).application, APP)
    assert.equal(path.dirname(output), temporary)
    assert.equal(path.dirname(path.resolve(temporary)), path.resolve(os.tmpdir()))
    assert.ok(path.basename(temporary).startsWith('saas-phase8-work-orders-browser-'))
    rmSync(temporary, { recursive: true, force: true })
  }
}
