/** Production Vue/Vite in the real host's opaque iframe -> Nginx -> independent Rust -> owned PostgreSQL. */
import assert from 'node:assert/strict'
import { createHash, randomUUID } from 'node:crypto'
import { existsSync, mkdirSync, readFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { launchBrowser, browserName, playwrightVersion, playwright } from './browser/runtime.mjs'
import { startPhase21Services, APP, API, delay, wait, repository } from './helpers/phase21-work-orders-services.mjs'

export const PHASE21_BROWSER_CHECKS = [
  'production-vite-module-and-dynamic-chunk', 'opaque-origin-null-rust-no-cookie', 'ordinary-user-real-authorization',
  'theme-no-extra-list-request', 'real-create-assignment-directory', 'real-update-conflict-preserves-draft',
  'explicit-conflict-rebase', 'real-status-filter', 'csv-host-mediated-download', 'reminder-inbox-authoritative',
  'reminder-committed-response-loss-recovery', 'create-committed-response-loss-recovery',
  'create-detail-menu-isolation', 'read-revocation-clears-server-content', 'real-soft-delete',
]
const sourceSha256 = createHash('sha256').update(readFileSync(fileURLToPath(import.meta.url))).digest('hex')
const checks = [], evidence = {}, contexts = [], operations = [], assets = [], errors = [], requestMetadata = new Map()
const nativeAttempts = new Map(), observationErrors = [], headerTasks = new Set(), faults = []
const uuid = value => typeof value === 'string' && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value)
let services, browser, browserClosed = false, failure, cleanup
function check(id) {
  assert(PHASE21_BROWSER_CHECKS.includes(id) && !checks.some(item => item.id === id), 'Unexpected or duplicate Phase21 check')
  checks.push({ id, status: 'PASS' }); console.log('PASS phase21 browser ' + id)
}
async function navigate(page, pathname) {
  await page.evaluate(next => { history.pushState(null, '', next); dispatchEvent(new PopStateEvent('popstate')) }, pathname)
  await page.waitForURL(url => url.pathname === pathname)
}
const frameFor = page => page.frameLocator('.micro-mount-zone iframe')
const route = id => '/micro/' + APP + (id ? '/tickets/' + id : '')
async function open(actor, pathname = route()) {
  const context = await browser.newContext({ viewport: { width: 1540, height: 1100 }, acceptDownloads: true })
  contexts.push(context)
  await context.exposeBinding('__phase21ObserveBusinessFetch', (_source, observation) => {
    if (!uuid(observation?.requestId)) { observationErrors.push('native-request-id-invalid'); return }
    const attempt = nativeAttempts.get(observation.requestId) || { requestId: observation.requestId }
    nativeAttempts.set(observation.requestId, attempt)
    if (observation.phase === 'start') {
      if (attempt.started) observationErrors.push('duplicate-native-request-id')
      Object.assign(attempt, { started: true, method: observation.method, path: observation.path, credentials: observation.credentials,
        signalAborted: Boolean(attempt.signalAborted || observation.signalAborted), nativeHeaders: observation.nativeHeaders })
    } else if (observation.phase === 'abort') attempt.signalAborted = true
    else if (observation.phase === 'settled') {
      if (attempt.nativeOutcome) observationErrors.push('duplicate-native-settlement')
      Object.assign(attempt, { nativeOutcome: observation.nativeOutcome, nativeErrorName: observation.nativeErrorName,
        signalAborted: Boolean(attempt.signalAborted || observation.signalAborted) })
    } else observationErrors.push('unknown-native-observation-phase')
  })
  // Observe the actual SDK's fetch options without changing the URL, options,
  // signal, request, response or native completion behavior. No credentials,
  // Authorization header, token or body enters this test metadata channel.
  await context.addInitScript(() => {
    if (window.top === window) return
    const nativeFetch = window.fetch.bind(window)
    window.fetch = function (input, options) {
      let event, signal, abort
      const emit = value => { try { void window.__phase21ObserveBusinessFetch(value).catch(() => {}) } catch {} }
      try {
        const pathname = new URL(typeof input === 'string' || input instanceof URL ? input : input.url, location.href).pathname
        if (pathname.startsWith('/work-orders-api/')) {
          // Headers cloning reads metadata only; never construct a Request,
          // which could transfer/consume a caller's existing request body.
          const headers = new Headers(options?.headers || (input instanceof Request ? input.headers : undefined))
          signal = options?.signal || (input instanceof Request ? input.signal : undefined)
          event = { requestId: headers.get('X-Request-Id'), method: String(options?.method || (input instanceof Request ? input.method : 'GET')).toUpperCase(),
            path: pathname, credentials: options?.credentials ?? null,
            nativeHeaders: { cookiePresent: Boolean(headers.get('Cookie')), bearerPresent: /^Bearer .+/.test(headers.get('Authorization') || ''),
              requestIdPresent: /^[0-9a-f-]{36}$/i.test(headers.get('X-Request-Id') || '') } }
          emit({ ...event, phase: 'start', signalAborted: Boolean(signal?.aborted) })
          abort = () => emit({ requestId: event.requestId, phase: 'abort' })
          signal?.addEventListener('abort', abort, { once: true })
        }
      } catch { /* Observation cannot alter native fetch error behavior. */ }
      const settled = (outcome, error) => {
        if (!event) return
        signal?.removeEventListener('abort', abort)
        emit({ requestId: event.requestId, phase: 'settled', nativeOutcome: outcome,
          nativeErrorName: error ? (/^[A-Za-z]{1,40}$/.test(error.name || '') ? error.name : 'UnknownError') : null,
          signalAborted: Boolean(signal?.aborted) })
      }
      let nativePromise
      try { nativePromise = nativeFetch(input, options) }
      catch (error) { settled('rejected', error); throw error }
      // Observe a side branch and return the identical native Promise. Neither
      // settlement callback rethrows into an unhandled observer rejection.
      void nativePromise.then(() => settled('fulfilled'), error => settled('rejected', error)).catch(() => {})
      return nativePromise
    }
  })
  await context.addCookies([{ name: 'phase21-cookie-must-not-reach-business', value: 'opaque-credential-observer', url: services.origin, httpOnly: true, secure: true }])
  await context.addInitScript(({ session, app }) => {
    if (window.top !== window) return
    localStorage.setItem('platform-session-v1', JSON.stringify({ schema: 1, version: 1, sessionId: crypto.randomUUID(), status: 'active', token: session, refreshToken: null }))
    localStorage.setItem('platform-current-app', app)
    localStorage.setItem('platform-theme', 'light')
  }, { session: actor.session, app: services.application.id })
  const page = await context.newPage(); page.setDefaultTimeout(20_000)
  page.on('pageerror', error => errors.push(error.message))
  page.on('request', request => {
    const url = new URL(request.url())
    if (!url.pathname.startsWith('/work-orders-api/')) return
    let frameType = 'unavailable', framePath = null
    try { const frame = request.frame(); frameType = frame === page.mainFrame() ? 'host' : 'iframe'; framePath = new URL(frame.url()).pathname }
    catch { /* Worker or already-retired frame ownership remains explicit. */ }
    const metadata = { method: request.method(), path: url.pathname, frameType, framePath, resourceType: request.resourceType(),
      outcome: 'pending', responseStatus: null, failureCode: null }
    requestMetadata.set(request, metadata)
    const task = (async () => {
      const headers = await request.allHeaders()
      Object.assign(metadata, { requestId: headers['x-request-id'] ?? null, query: url.search, origin: headers.origin ?? null,
        cookiePresent: Boolean(headers.cookie), bearerPresent: Boolean(headers.authorization?.startsWith('Bearer ')),
        requestIdPresent: uuid(headers['x-request-id']), key: headers['idempotency-key'] ?? null, body: request.postData() ?? null })
      operations.push(metadata)
    })()
    headerTasks.add(task)
    void task.then(() => headerTasks.delete(task), () => { headerTasks.delete(task); observationErrors.push('browser-headers-unobserved') })
  })
  page.on('requestfinished', request => { const metadata = requestMetadata.get(request); if (metadata) metadata.outcome = 'finished' })
  page.on('requestfailed', request => { const metadata = requestMetadata.get(request); if (metadata) {
    metadata.outcome = 'failed'
    const code = request.failure()?.errorText || ''
    metadata.failureCode = /^[A-Za-z0-9_:.-]{1,100}$/.test(code) ? code : 'UNAVAILABLE'
  } })
  page.on('response', async response => {
    const metadata = requestMetadata.get(response.request()); if (metadata) metadata.responseStatus = response.status()
    const url = new URL(response.url())
    if (!url.pathname.startsWith('/sub-apps/work-orders/versions/phase21/')) return
    assets.push({ path: url.pathname, status: response.status(), contentType: response.headers()['content-type'] || '',
      allowOrigin: response.headers()['access-control-allow-origin'] ?? null,
      origin: response.request().headers().origin ?? null, resourceType: response.request().resourceType() })
  })
  await page.goto(services.origin + pathname, { waitUntil: 'domcontentloaded' })
  return { context, page, frame: frameFor(page) }
}
async function active(view) {
  await view.frame.locator('[data-testid="work-orders-app"][data-context-active="true"]').waitFor()
  await view.page.getByTestId('micro-status-bar').getByText('运行中', { exact: true }).waitFor()
}
async function detail(view, id, title) {
  await view.frame.locator('[data-testid="ticket-detail"] [data-ticket-id="' + id + '"]').waitFor()
  if (title) await view.frame.getByTestId('ticket-detail').getByRole('heading', { name: title, exact: true }).waitFor()
}
async function business(actor, endpoint = API, options = {}, status = 200) {
  const result = await services.businessCall(actor, endpoint, options)
  assert.equal(result.status, status, 'Real Rust API contract status differs: ' + (result.data?.code || result.status))
  return result
}
async function snapshotCreate(key) {
  const receipts = (await services.businessDb.query('SELECT ticket_id,response_json,request_hash FROM work_order_create_idempotency WHERE idempotency_key=$1', [key])).rows
  assert.equal(receipts.length, 1, 'Same create event must persist one receipt')
  const tickets = (await services.businessDb.query('SELECT id,version,title,deleted_at FROM work_orders WHERE id=$1', [receipts[0].ticket_id])).rows
  assert.equal(tickets.length, 1)
  return { receiptCount: receipts.length, ticketCount: tickets.length, ticket: tickets[0] }
}
async function snapshotReminder(key) {
  const rows = (await services.platformDb.query(`SELECT p.id AS publication_id,n.id AS notification_id,n.user_id,n.tenant_id,n.organization_id,n.link
    FROM notification_publications p LEFT JOIN notifications n ON n.publication_id=p.id WHERE p.idempotency_key=$1`, [key])).rows
  assert.equal(rows.length, 1, 'A reminder event has exactly one real publication/delivery')
  assert(rows[0].notification_id, 'A durable inbox row must exist')
  return rows[0]
}
async function dropCommittedResponse(view, endpoint, expected, action) {
  let observed, handlerError, claimed = false
  const matcher = url => new URL(url).pathname === endpoint
  const handler = async route => {
    try {
    const request = route.request()
    if (request.method() !== 'POST' || new URL(request.url()).pathname !== endpoint || claimed) return await route.continue()
    claimed = true
    const originalHeaders = await request.allHeaders()
    assert.equal(originalHeaders.origin, 'null')
    assert.equal(Boolean(originalHeaders.cookie), false)
    assert(originalHeaders.authorization?.startsWith('Bearer ') && uuid(originalHeaders['x-request-id']))
    // An isolated API context starts with an empty cookie jar. context.route's
    // own API context would otherwise inject the browser's sentinel cookie.
    const forwarding = await playwright.request.newContext()
    const fault = { kind: endpoint.endsWith('/remind') ? 'reminder' : 'create', requestId: originalHeaders['x-request-id'],
      upstreamStatus: null, freshCookieJar: false, forwardedOrigin: originalHeaders.origin, forwardedCookiePresent: false,
      forwardedBearerPresent: Boolean(originalHeaders.authorization?.startsWith('Bearer ')), forwardedRequestIdPresent: uuid(originalHeaders['x-request-id']),
      forwardedRequestId: originalHeaders['x-request-id'], normalTls: true, disposed: false }
    faults.push(fault)
    let committed
    try {
      const before = await forwarding.storageState()
      const browserCookies = await view.context.cookies(services.origin)
      fault.freshCookieJar = before.cookies.length === 0 && browserCookies.some(cookie => cookie.name === 'phase21-cookie-must-not-reach-business')
      assert(fault.freshCookieJar, 'The fault forwarder must be isolated from the real browser cookie jar')
      const headers = Object.fromEntries(Object.entries(originalHeaders).filter(([name]) => !name.startsWith(':') &&
        !['connection', 'keep-alive', 'proxy-connection', 'transfer-encoding', 'upgrade', 'host', 'content-length'].includes(name.toLowerCase())))
      Object.assign(fault, { forwardedOrigin: headers.origin, forwardedCookiePresent: Boolean(headers.cookie),
        forwardedBearerPresent: Boolean(headers.authorization?.startsWith('Bearer ')), forwardedRequestIdPresent: uuid(headers['x-request-id']),
        forwardedRequestId: headers['x-request-id'] })
      for (const name of ['origin', 'authorization', 'x-request-id', 'idempotency-key', 'content-type']) assert.equal(headers[name], originalHeaders[name])
      const response = await forwarding.fetch(request, { headers, maxRetries: 0, maxRedirects: 0 })
      fault.upstreamStatus = response.status()
      assert.equal(response.status(), expected, 'The loss fault must follow a real successful commit response')
      assert.equal((await forwarding.storageState()).cookies.length, 0, 'The isolated fault forwarder must remain cookie-free')
      committed = { status: response.status(), body: request.postData(), key: originalHeaders['idempotency-key'] ?? null,
        response: await response.json(), upstreamCommitted: true }
    } finally { await forwarding.dispose(); fault.disposed = true }
    await route.abort('connectionreset')
    // Publishing completion before abort allowed the caller to unroute an active handler.
    observed = committed
    } catch (error) { handlerError = error }
  }
  await view.context.route(matcher, handler)
  try {
    await action(); await wait(() => observed || handlerError, 'Real committed response must reach the deliberate loss fault')
    if (handlerError) throw handlerError
  }
  finally { await view.context.unroute(matcher, handler) }
  return observed
}
async function screenshot(page, name) {
  if (!process.env.WORK_ORDERS_BROWSER_REPORT_DIR) return
  const directory = path.resolve(process.env.WORK_ORDERS_BROWSER_REPORT_DIR)
  assert(path.isAbsolute(process.env.WORK_ORDERS_BROWSER_REPORT_DIR), 'Report directory must be absolute')
  assert(!directory.startsWith(repository + path.sep) || directory.startsWith(path.join(repository, 'target') + path.sep), 'Screenshots cannot overwrite source inputs')
  mkdirSync(directory, { recursive: true })
  const destination = path.join(directory, name + '.png'); assert(!existsSync(destination), 'Screenshot output must be fresh')
  await page.screenshot({ path: destination, fullPage: true })
  evidence.screenshots ??= []
  evidence.screenshots.push({ path: destination, sha256: createHash('sha256').update(readFileSync(destination)).digest('hex') })
}
function transportProof(actual) {
  assert.deepEqual(observationErrors, [], 'Every transport observation must be complete')
  const events = new Map(), faultById = new Map()
  for (const event of actual) {
    assert(uuid(event.requestId) && !events.has(event.requestId), 'Each browser request event needs one unique actual SDK request ID')
    events.set(event.requestId, event)
    assert(nativeAttempts.has(event.requestId), 'Every browser business request must match a real native SDK fetch')
    assert.equal(event.frameType, 'iframe')
    assert(event.framePath?.startsWith('/sub-apps/work-orders/versions/phase21/'))
    assert.equal(event.cookiePresent, false)
    assert(event.bearerPresent && event.requestIdPresent)
    // A missing header on an aborted provisional Chromium event is retained as
    // unconfirmed metadata. An observed non-opaque Origin is always a failure.
    if (event.origin !== null) assert.equal(event.origin, 'null')
  }
  assert.equal(faults.length, 2)
  assert.deepEqual(faults.map(fault => fault.kind).sort(), ['create', 'reminder'])
  for (const fault of faults) {
    assert(uuid(fault.requestId) && !faultById.has(fault.requestId))
    faultById.set(fault.requestId, fault)
    assert(fault.freshCookieJar && !fault.forwardedCookiePresent && fault.forwardedBearerPresent && fault.forwardedRequestIdPresent && fault.normalTls && fault.disposed)
    assert.equal(fault.forwardedOrigin, 'null'); assert.equal(fault.forwardedRequestId, fault.requestId)
    assert.equal(fault.upstreamStatus, fault.kind === 'create' ? 201 : 200)
  }
  const records = []
  for (const attempt of nativeAttempts.values()) {
    assert(attempt.started && uuid(attempt.requestId) && attempt.nativeOutcome, 'Every native attempt must have an observed start and settlement')
    assert.equal(attempt.credentials, 'omit')
    assert(attempt.nativeHeaders?.bearerPresent && attempt.nativeHeaders.requestIdPresent && !attempt.nativeHeaders.cookiePresent)
    const event = events.get(attempt.requestId), fault = faultById.get(attempt.requestId)
    if (event) { assert.equal(event.method, attempt.method); assert.equal(event.path, attempt.path) }
    const cancelled = attempt.method === 'GET' && attempt.signalAborted && attempt.nativeOutcome === 'rejected' && attempt.nativeErrorName === 'AbortError'
    let classification
    if (fault) {
      assert(event && event.method === 'POST' && event.outcome === 'failed' && event.responseStatus === null && event.failureCode === 'net::ERR_CONNECTION_RESET')
      assert.equal(event.origin, 'null')
      assert(attempt.nativeOutcome === 'rejected' && attempt.nativeErrorName === 'TypeError')
      classification = 'fixture-response-drop'
    } else if (event?.responseStatus !== null && event?.responseStatus !== undefined) {
      assert.equal(event.origin, 'null')
      assert(attempt.nativeOutcome === 'fulfilled' && attempt.nativeErrorName === null)
      if (event.outcome === 'finished') {
        assert.equal(event.failureCode, null)
        classification = 'browser-response'
      } else {
        // The SDK intentionally cancels unread non-OK bodies instead of
        // retaining remote error text. Fetch has already fulfilled with real
        // HTTP response headers, so this body abort still confirms the wire.
        assert(Number.isInteger(event.responseStatus) && event.responseStatus >= 400 && event.responseStatus <= 599 &&
          event.outcome === 'failed' && event.failureCode === 'net::ERR_ABORTED',
        'Only an observed HTTP error response followed by its body cancellation may retain a failed terminal browser event')
        classification = 'browser-error-body-cancelled'
      }
    } else if (!event) {
      assert(cancelled, 'A native attempt without a browser event requires observed GET signal cancellation and native AbortError')
      classification = 'aborted-before-request-event'
    } else {
      assert(cancelled && event.outcome === 'failed' && event.responseStatus === null && event.failureCode === 'net::ERR_ABORTED',
        'A provisional browser event requires its paired native GET cancellation and actual ERR_ABORTED; unknown failures cannot be discarded')
      classification = 'aborted-without-wire-confirmation'
    }
    const browserEvent = event ? { requestId: event.requestId, method: event.method, path: event.path, origin: event.origin,
      cookiePresent: event.cookiePresent, bearerPresent: event.bearerPresent, requestIdPresent: event.requestIdPresent,
      frameType: event.frameType, framePath: event.framePath, outcome: event.outcome, responseStatus: event.responseStatus, failureCode: event.failureCode } : null
    records.push({ requestId: attempt.requestId, method: attempt.method, path: attempt.path, credentials: attempt.credentials,
      signalAborted: attempt.signalAborted, nativeOutcome: attempt.nativeOutcome, nativeErrorName: attempt.nativeErrorName,
      nativeHeaders: attempt.nativeHeaders, browserEvent, classification })
  }
  assert(faults.every(fault => records.some(record => record.requestId === fault.requestId && record.classification === 'fixture-response-drop')))
  const count = classification => records.filter(record => record.classification === classification).length
  const responseBodyCancelledCount = count('browser-error-body-cancelled')
  const proof = { nativeAttemptCount: records.length, browserRequestEventCount: actual.length,
    confirmedWireCount: count('browser-response') + responseBodyCancelledCount, responseBodyCancelledCount,
    abortBeforeRequestEventCount: count('aborted-before-request-event'), abortedWithoutWireConfirmationCount: count('aborted-without-wire-confirmation'),
    faultInjectedCount: count('fixture-response-drop'), records, faults }
  assert.equal(proof.nativeAttemptCount, proof.confirmedWireCount + proof.abortBeforeRequestEventCount + proof.abortedWithoutWireConfirmationCount + proof.faultInjectedCount)
  assert.equal(proof.browserRequestEventCount, proof.nativeAttemptCount - proof.abortBeforeRequestEventCount)
  return proof
}
function logTransportDiagnostic(drainComplete) {
  const actual = operations.filter(item => item.method !== 'OPTIONS')
  console.log('PHASE21_TRANSPORT_DIAGNOSTIC ' + JSON.stringify({ drainComplete,
    operations: actual.map(item => ({ requestId: item.requestId, method: item.method, path: item.path, origin: item.origin,
      cookiePresent: item.cookiePresent, bearerPresent: item.bearerPresent, requestIdPresent: item.requestIdPresent,
      frameType: item.frameType, framePath: item.framePath, resourceType: item.resourceType,
      outcome: item.outcome, responseStatus: item.responseStatus, failureCode: item.failureCode })),
    nativeAttempts: [...nativeAttempts.values()].map(attempt => ({ requestId: attempt.requestId, method: attempt.method, path: attempt.path,
      credentials: attempt.credentials, signalAborted: attempt.signalAborted, nativeOutcome: attempt.nativeOutcome,
      nativeErrorName: attempt.nativeErrorName, nativeHeaders: attempt.nativeHeaders })), observationErrors }))
  return actual
}

try {
  services = await startPhase21Services()
  browser = await launchBrowser()
  evidence.browser = { product: browserName, version: await browser.version(), playwrightVersion }
  const actor = services.actors.ordinary, peer = services.actors.peer
  const profile = await services.platformApi('/api/auth/profile', { token: actor.session })
  assert.equal(profile.user.isSuperAdmin, false)
  const context = await services.platformApi('/api/apps/work-orders/context', { token: actor.token })
  assert(context.permissions.includes('ticket:read') && context.permissions.includes('notification:publish'))
  assert(!context.permissions.some(value => value.startsWith('system:')))
  evidence.ordinary = { superAdmin: false, managementPermissions: false, directMembership: true }
  check('ordinary-user-real-authorization')
  const view = await open(actor); await active(view)
  const sandbox = await view.page.locator('.micro-mount-zone iframe').getAttribute('sandbox')
  assert.equal(sandbox, 'allow-scripts allow-forms')
  assert.equal(await view.frame.locator('body').evaluate(() => location.origin), services.origin)
  // A sandboxed document's URL origin differs from its effective opaque origin.
  // Browser HTTP Origin:null below is the authoritative transport observation.
  const buildInfo = services.source.viteBuild.find(item => item.path === 'build-info.json')
  assert(buildInfo, 'Actual Vite build metadata is part of the release')
  const manifest = JSON.parse(readFileSync(path.join(services.ownedRoot, 'app/build-info.json'), 'utf8'))
  const viteManifest = JSON.parse(readFileSync(path.join(services.ownedRoot, 'app/asset-manifest.json'), 'utf8'))
  assert(manifest.entry && Array.isArray(manifest.dynamicChunks) && manifest.dynamicChunks.length > 0)
  assert.equal(viteManifest['index.html'].file, manifest.entry)
  const dynamicImports = viteManifest['index.html'].dynamicImports
  assert(Array.isArray(dynamicImports) && dynamicImports.length > 0)
  assert(dynamicImports.every(key => viteManifest[key]?.isDynamicEntry === true && manifest.dynamicChunks.includes(viteManifest[key].file)),
    'Dynamic chunks must correspond to actual Vite dynamic import graph entries')
  await wait(() => [manifest.entry, ...manifest.dynamicChunks].every(asset => assets.some(item => item.path.endsWith('/' + asset) && item.status === 200)),
    'The real iframe must load its module and every startup dynamic chunk')
  const loaded = assets.filter(item => [manifest.entry, ...manifest.dynamicChunks].some(asset => item.path.endsWith('/' + asset)))
  assert(loaded.every(item => /javascript/.test(item.contentType) && ['*', 'null'].includes(item.allowOrigin) && item.origin === 'null'))
  assert.equal(await view.frame.locator('script[type="module"]').count(), 1)
  evidence.assets = { moduleEntry: manifest.entry, dynamicChunks: manifest.dynamicChunks, manifestDynamicImports: dynamicImports, loaded, sandbox, actualVueMounted: true }
  check('production-vite-module-and-dynamic-chunk')

  await wait(() => operations.some(item => item.method === 'GET' && item.path === API), 'Actual first Rust list request')
  const businessRequests = operations.filter(item => item.method !== 'OPTIONS')
  assert(businessRequests.every(item => item.origin === 'null' && !item.cookiePresent && item.bearerPresent && item.requestIdPresent))
  await wait(() => nativeAttempts.size > 0, 'Actual iframe fetch credentials options must be observed')
  assert([...nativeAttempts.values()].every(item => item.credentials === 'omit'), 'Every actual business fetch explicitly omits browser credentials')
  const preflight = await services.request(services.origin, API, { method: 'OPTIONS', headers: { Origin: 'null',
    'Access-Control-Request-Method': 'POST', 'Access-Control-Request-Headers': 'authorization,content-type,idempotency-key,x-request-id' } })
  assert.equal(preflight.status, 204)
  assert.equal(preflight.headers['access-control-allow-origin'], 'null')
  assert.equal(preflight.headers['access-control-allow-credentials'], undefined)
  assert.match(preflight.headers['access-control-allow-headers'], /idempotency-key/i)
  evidence.transport = { origin: 'null', cookiePresent: false, appBearerPresent: true, requestIdPresent: true, rustPreflightStatus: 204,
    credentialsAllowed: false, credentials: [...nativeAttempts.values()][0].credentials }
  check('opaque-origin-null-rust-no-cookie')

  const listCount = () => operations.filter(item => item.method === 'GET' && item.path === API).length
  const reads = listCount()
  await view.page.getByRole('button', { name: '切换到深色模式', exact: true }).click()
  await view.frame.locator('html[data-theme="dark"]').waitFor()
  await view.frame.locator('body').evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))))
  await delay(150)
  assert.equal(listCount(), reads, 'A theme notification cannot reload the real Rust list')
  evidence.theme = { before: reads, after: listCount() }; check('theme-no-extra-list-request')

  await view.frame.getByTestId('create-ticket').click()
  await view.page.waitForURL('**/micro/work-orders/ticket-create')
  await view.frame.getByTestId('ticket-title').fill('Phase21 ordinary real CRUD')
  await view.frame.getByTestId('ticket-description').fill('Created by actual Vite Vue UI through the Rust API')
  await view.frame.getByTestId('ticket-priority-input').selectOption('HIGH')
  await view.frame.getByTestId('ticket-assignee').locator('option[value="' + peer.userId + '"]').waitFor({ state: 'attached' })
  await view.frame.getByTestId('ticket-assignee').selectOption(peer.userId)
  const createResponse = view.page.waitForResponse(response => response.request().method() === 'POST' && new URL(response.url()).pathname === API)
  await view.frame.getByTestId('save-ticket').click()
  const createdResponse = await createResponse; assert.equal(createdResponse.status(), 201)
  let ticket = await createdResponse.json(); await detail(view, ticket.id, ticket.title)
  assert.equal(ticket.assigneeUserId, peer.userId)
  const createOperation = operations.findLast(item => item.method === 'POST' && item.path === API)
  assert.match(createOperation.key || '', /^[A-Za-z0-9._:-]{1,128}$/)
  const persistedCreate = await snapshotCreate(createOperation.key)
  assert.equal(persistedCreate.ticket.id, ticket.id)
  evidence.crud = { ticketId: ticket.id, createdStatus: 201, assignee: true, directoryRustRequest: operations.some(item => item.path === '/work-orders-api/assignees') }
  assert(evidence.crud.directoryRustRequest); check('real-create-assignment-directory')

  await view.frame.getByTestId('edit-ticket').click()
  const draft = 'Phase21 preserved conflict draft'
  await view.frame.getByTestId('ticket-title').fill(draft)
  const competing = await business(actor, API + '/' + ticket.id, { method: 'PATCH', body: { version: ticket.version, description: 'Concurrent real Rust winner' } })
  ticket = competing.data
  const conflictResponse = view.page.waitForResponse(response => response.request().method() === 'PATCH' && new URL(response.url()).pathname === API + '/' + ticket.id)
  await view.frame.getByTestId('save-ticket').click()
  assert.equal((await conflictResponse).status(), 409)
  await view.frame.getByTestId('version-conflict').waitFor()
  assert.equal(await view.frame.getByTestId('ticket-title').inputValue(), draft)
  assert(await view.frame.getByTestId('save-ticket').isDisabled())
  evidence.conflict = { status: 409, draftPreserved: true, automaticOverwrite: false }; check('real-update-conflict-preserves-draft')
  await view.frame.getByTestId('use-latest-version').click()
  assert.equal(await view.frame.getByTestId('ticket-title').inputValue(), draft)
  const saved = view.page.waitForResponse(response => response.request().method() === 'PATCH' && new URL(response.url()).pathname === API + '/' + ticket.id && response.status() === 200)
  await view.frame.getByTestId('save-ticket').click(); ticket = await (await saved).json()
  await detail(view, ticket.id, draft)
  evidence.conflict.explicitVersionAccepted = ticket.version; check('explicit-conflict-rebase')

  const statusResponse = view.page.waitForResponse(response => response.request().method() === 'PATCH' && new URL(response.url()).pathname === API + '/' + ticket.id)
  await view.frame.getByTestId('change-ticket-status').click(); ticket = await (await statusResponse).json()
  assert.equal(ticket.status, 'IN_PROGRESS')
  await detail(view, ticket.id)
  await view.frame.getByTestId('back-to-list').click()
  await view.frame.getByTestId('filter-status').selectOption('IN_PROGRESS')
  await view.frame.getByTestId('filter-priority').selectOption('HIGH')
  await view.frame.locator('[data-testid="ticket-row"][data-ticket-id="' + ticket.id + '"]').waitFor()
  evidence.crud.status = ticket.status; evidence.crud.filterVerified = true; check('real-status-filter')

  const downloadEvent = view.page.waitForEvent('download')
  await view.frame.getByTestId('export-tickets').click()
  const download = await downloadEvent
  const file = await download.path(), csv = readFileSync(file, 'utf8')
  assert(csv.includes(ticket.number) && csv.includes(draft))
  assert.equal(await view.page.locator('.micro-mount-zone iframe').getAttribute('sandbox'), 'allow-scripts allow-forms')
  await download.delete()
  evidence.crud.csv = { realRustExport: operations.some(item => item.path === API + '/export.csv'), hostDownload: true, sandboxUnchanged: true }
  assert(evidence.crud.csv.realRustExport); check('csv-host-mediated-download')

  await view.frame.locator('[data-testid="ticket-row"][data-ticket-id="' + ticket.id + '"]').getByTestId('open-ticket').click()
  await detail(view, ticket.id)
  const reminderResponse = view.page.waitForResponse(response => response.request().method() === 'POST' && new URL(response.url()).pathname === API + '/' + ticket.id + '/remind')
  await view.frame.getByTestId('remind-ticket').click()
  const receiptResponse = await reminderResponse; assert.equal(receiptResponse.status(), 200)
  const receipt = await receiptResponse.json()
  const reminderOperation = operations.findLast(item => item.method === 'POST' && item.path.endsWith('/remind'))
  const key = JSON.parse(reminderOperation.body).idempotencyKey
  const notification = await snapshotReminder(key)
  assert.equal(notification.user_id, peer.userId)
  assert.equal(notification.publication_id, receipt.publicationId)
  const inbox = await services.platformApi('/api/notifications?page=1&size=100', { token: peer.session })
  assert(inbox.records.some(row => row.id === notification.notification_id && row.link === route(ticket.id)))
  const afterReminder = (await business(actor, API + '/' + ticket.id)).data
  assert.equal(afterReminder.version, ticket.version)
  evidence.reminder = { publicationCount: 1, deliveryCount: 1, recipientInboxVerified: true, ticketVersionUnchanged: true, realtimeStarted: false }
  check('reminder-inbox-authoritative')

  const reminderLost = await dropCommittedResponse(view, API + '/' + ticket.id + '/remind', 200, () => view.frame.getByTestId('remind-ticket').click())
  await view.frame.getByTestId('reminder-recovery').waitFor()
  const lostReminderKey = JSON.parse(reminderLost.body).idempotencyKey
  await snapshotReminder(lostReminderKey)
  const sendsBeforeRecovery = operations.filter(item => item.method === 'POST' && item.path.endsWith('/remind')).length
  await delay(200)
  assert.equal(operations.filter(item => item.method === 'POST' && item.path.endsWith('/remind')).length, sendsBeforeRecovery)
  const recoverReminder = view.page.waitForResponse(response => response.request().method() === 'POST' && new URL(response.url()).pathname.endsWith('/remind'))
  await view.frame.getByTestId('remind-ticket').click()
  const recoveredReceipt = await (await recoverReminder).json()
  assert.equal(recoveredReceipt.publicationId, reminderLost.response.publicationId)
  assert.equal(recoveredReceipt.duplicate, true)
  const reminderRetry = operations.findLast(item => item.method === 'POST' && item.path.endsWith('/remind'))
  assert.equal(reminderRetry.body, reminderLost.body)
  await snapshotReminder(lostReminderKey)
  evidence.reminderRecovery = { committedResponseDropped: true, sameBody: true, sameKey: true, publicationCount: 1, deliveryCount: 1, automaticRetryCount: 0 }
  check('reminder-committed-response-loss-recovery')

  await navigate(view.page, '/micro/work-orders/ticket-create')
  await view.frame.getByTestId('ticket-title').fill('Phase21 committed response loss')
  const createLost = await dropCommittedResponse(view, API, 201, () => view.frame.getByTestId('save-ticket').click())
  await view.frame.getByTestId('create-recovery').waitFor()
  const pendingCreate = await snapshotCreate(createLost.key)
  assert(await view.frame.getByTestId('ticket-title').isDisabled())
  assert(await view.frame.getByTestId('save-ticket').isDisabled())
  const attemptsBeforeRecovery = operations.filter(item => item.method === 'POST' && item.path === API).length
  await delay(200)
  assert.equal(operations.filter(item => item.method === 'POST' && item.path === API).length, attemptsBeforeRecovery)
  const recoveryResponse = view.page.waitForResponse(response => response.request().method() === 'POST' && new URL(response.url()).pathname === API)
  await view.frame.getByTestId('recover-create').click()
  const recoveredResponse = await recoveryResponse; assert.equal(recoveredResponse.status(), 201)
  const recoveredTicket = await recoveredResponse.json()
  assert.equal(recoveredTicket.id, pendingCreate.ticket.id)
  await detail(view, recoveredTicket.id, 'Phase21 committed response loss')
  const retry = operations.findLast(item => item.method === 'POST' && item.path === API)
  assert.equal(retry.key, createLost.key); assert.equal(retry.body, createLost.body)
  const recoveredSnapshot = await snapshotCreate(createLost.key)
  assert.equal((await services.businessDb.query('SELECT count(*)::int AS count FROM work_orders WHERE title=$1', [recoveredTicket.title])).rows[0].count, 1)
  evidence.createRecovery = { committedResponseDropped: true, sameKey: true, sameBody: true, receiptCount: recoveredSnapshot.receiptCount,
    ticketCount: recoveredSnapshot.ticketCount, automaticRetryCount: 0 }
  check('create-committed-response-loss-recovery')

  const createOnly = await open(services.actors.createOnly, '/micro/work-orders/ticket-create'); await active(createOnly)
  await createOnly.frame.getByTestId('ticket-title').waitFor()
  const createActorContext = await services.platformApi('/api/apps/work-orders/context', { token: services.actors.createOnly.token })
  assert(createActorContext.permissions.includes('ticket:create') && !createActorContext.permissions.includes('ticket:read'))
  assert.equal((await services.businessCall(services.actors.createOnly, API + '/' + ticket.id)).status, 403)
  await navigate(createOnly.page, route(ticket.id)); await createOnly.page.getByText('无访问权限', { exact: true }).waitFor()
  assert.equal(await createOnly.frame.locator('[data-ticket-id="' + ticket.id + '"]').count(), 0)
  const readOnly = await open(services.actors.readOnly, route(ticket.id)); await active(readOnly); await detail(readOnly, ticket.id)
  assert.equal(await readOnly.frame.getByTestId('create-ticket').count(), 0)
  const readActorContext = await services.platformApi('/api/apps/work-orders/context', { token: services.actors.readOnly.token })
  assert(readActorContext.permissions.includes('ticket:read') && !readActorContext.permissions.includes('ticket:create'))
  assert.equal((await services.businessCall(services.actors.readOnly, API, { method: 'POST', headers: { 'Idempotency-Key': randomUUID() }, body: { title: 'Must be forbidden' } })).status, 403)
  await navigate(readOnly.page, '/micro/work-orders/ticket-create'); await readOnly.page.getByText('无访问权限', { exact: true }).waitFor()
  assert.equal(await readOnly.frame.getByTestId('ticket-editor').count(), 0)
  evidence.menuIsolation = { createCannotRead: true, readCannotCreate: true, actualContextAndRust403: true }
  check('create-detail-menu-isolation')

  await navigate(view.page, route()); await view.frame.locator('[data-testid="ticket-row"][data-ticket-id="' + ticket.id + '"]').waitFor()
  await services.setGrants(actor, true)
  const deniedRefresh = view.page.waitForResponse(response => response.request().method() === 'GET' && new URL(response.url()).pathname === API)
  await view.frame.getByTestId('refresh-tickets').click(); assert.equal((await deniedRefresh).status(), 403)
  await wait(async () => await view.frame.getByTestId('ticket-row').count() === 0, 'Read refusal clears previously served ticket rows')
  assert.equal(await view.frame.getByText(draft, { exact: true }).count(), 0)
  evidence.revocation = { actualRustStatus: 403, oldRowsVisible: false }; check('read-revocation-clears-server-content')
  await services.setGrants(actor)
  // Reload obtains a fresh real authorized context; no test mutates Vue state.
  await view.page.reload({ waitUntil: 'domcontentloaded' }); await active(view)
  await navigate(view.page, route(ticket.id)); await detail(view, ticket.id)
  await screenshot(view.page, 'phase21-real-ticket')
  await view.frame.getByTestId('delete-ticket').click()
  const deletedResponse = view.page.waitForResponse(response => response.request().method() === 'DELETE' && new URL(response.url()).pathname === API + '/' + ticket.id)
  await view.frame.getByTestId('confirm-delete').click(); assert.equal((await deletedResponse).status(), 200)
  assert.equal((await services.businessCall(actor, API + '/' + ticket.id)).status, 404)
  assert((await services.businessDb.query('SELECT deleted_at FROM work_orders WHERE id=$1', [ticket.id])).rows[0].deleted_at)
  await view.frame.getByTestId('ticket-list').waitFor()
  await view.frame.locator('[data-testid="ticket-row"][data-ticket-id="' + recoveredTicket.id + '"]').waitFor()
  evidence.crud.softDeleted = true; check('real-soft-delete')

  assert.equal(checks.length, PHASE21_BROWSER_CHECKS.length)
  assert.deepEqual(errors, [], 'Actual host and iframe cannot produce uncaught browser errors')
  try {
    await wait(() => headerTasks.size === 0 && [...nativeAttempts.values()].every(attempt => attempt.nativeOutcome) &&
      [...requestMetadata.values()].filter(event => event.method !== 'OPTIONS').every(event => event.outcome !== 'pending'),
    'Native fetch settlements and asynchronous browser headers/request outcomes must drain before transport classification')
  } catch (error) { logTransportDiagnostic(false); throw error }
  const actual = logTransportDiagnostic(true)
  const proof = transportProof(actual)
  Object.assign(evidence.transport, proof, { actualRequestCount: proof.confirmedWireCount + proof.faultInjectedCount,
    explicitOmitObservationCount: proof.nativeAttemptCount })
} catch (error) {
  failure = error
  console.error('Phase21 browser failed: ' + (services?.redact(error.message) || String(error.message).replace(/Bearer\s+[^\s"']+/gi, 'Bearer [redacted]').replace(/postgres(?:ql)?:\/\/[^\s"']+/g, '[database-url-redacted]')))
  process.exitCode = 1
} finally {
  for (const context of contexts) await context.close().catch(() => { process.exitCode = 1 })
  if (browser) { await browser.close().then(() => { browserClosed = true }, () => { process.exitCode = 1 }) }
  else browserClosed = true
  if (services) cleanup = await services.close({ browserClosed }).catch(error => { process.exitCode = 1; return error.phase21Cleanup || { failed: true } })
  cleanup = { ...(cleanup || failure?.phase21StartupCleanup), browserClosed }
  const result = { schemaVersion: 1, status: !failure && !process.exitCode && checks.length === PHASE21_BROWSER_CHECKS.length ? 'PASS' : 'FAIL',
    mode: 'actual-production-host-vite-opaque-rust-postgres-nginx', product: browserName, sourceSha256,
    passed: checks.length, failed: failure || process.exitCode ? 1 : 0, skipped: 0,
    checks, evidence, source: services?.source, cleanup }
  console.log('PHASE21_BROWSER_RESULT ' + JSON.stringify(result))
  if (result.status !== 'PASS') process.exitCode = 1
}
