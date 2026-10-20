/** Phase 23 real browser consumer. The caller owns isolated PostgreSQL and both
 * Rust services. Node starts Vite and drives the browser only. No HTTP business
 * response or Socket.IO frame is synthesized by this test. */
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { createHash, randomUUID } from 'node:crypto'
import { mkdirSync, readFileSync } from 'node:fs'
import net from 'node:net'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { browserName, launchBrowser, playwrightVersion } from './browser/runtime.mjs'
import { commandEnvironment } from '../scripts/ci/report.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const delay = ms => new Promise(resolve => setTimeout(resolve, ms))
const deferred = () => {
  const gate = { resolved: false }
  gate.promise = new Promise(done => { gate.resolve = () => { gate.resolved = true; done() } })
  return gate
}
async function wait(predicate, description, timeout = 25_000) {
  const deadline = Date.now() + timeout
  while (Date.now() < deadline) { if (await predicate()) return; await delay(25) }
  assert(await predicate(), description)
}

/** Reserve the exact same-origin browser URL before the caller starts the Rust
 * notification service with its explicit allowed-origin list. Vite uses strictPort. */
export async function reservePhase23BrowserOrigin() {
  const port = await new Promise((resolve, reject) => {
    const listener = net.createServer().once('error', reject)
    listener.listen(0, '127.0.0.1', () => {
      const port = listener.address().port
      listener.close(error => error ? reject(error) : resolve(port))
    })
  })
  return { origin: `http://127.0.0.1:${port}`, port }
}

// Fault injection retains real transports: save native WebSockets for a network
// outage, and detach one native HTTP abort to let the production session fence
// reject a genuinely late server response. Never fabricate API or socket data.
function observeNativeTransports({ origin, session }) {
  if (location.origin !== origin) return
  if (!localStorage.getItem('platform-session-v1')) {
    localStorage.setItem('platform-session-v1', JSON.stringify({ schema: 1, version: 1,
      sessionId: crypto.randomUUID(), status: 'active', token: session.accessToken, refreshToken: session.refreshToken }))
  }
  localStorage.setItem('platform-theme', 'light')
  window.__phase23Document = crypto.randomUUID()
  const trace = window.__phase23Transport = { requests: [], authentications: [], detachNextInbox: false }
  const fetchNative = window.fetch.bind(window)
  window.fetch = (input, options = {}) => {
    const url = new URL(typeof input === 'string' ? input : input.url, location.href)
    if (!url.pathname.startsWith('/api/notifications')) return fetchNative(input, options)
    const detached = trace.detachNextInbox && url.pathname === '/api/notifications' && (!options.method || options.method === 'GET')
    if (detached) trace.detachNextInbox = false
    const row = { path: url.pathname, started: Date.now(), detached, aborted: Boolean(options.signal?.aborted),
      organizationId: window.__phase23Auth?.getState().profile?.organization.id ?? null }
    trace.requests.push(row)
    options.signal?.addEventListener('abort', () => { row.aborted = true }, { once: true })
    return fetchNative(input, detached ? { ...options, signal: undefined } : options).then(response => {
      row.status = response.status; row.completed = Date.now(); return response
    }, error => { row.failed = true; throw error })
  }
  const sockets = new Set()
  window.WebSocket = new Proxy(window.WebSocket, { construct(target, args) {
    const socket = Reflect.construct(target, args)
    // Vite HMR is a separate native socket. Closing it could reload the page
    // and falsely make a fresh document look like notification reconnection.
    if (new URL(socket.url, location.href).pathname !== '/socket.io/') return socket
    sockets.add(socket); socket.addEventListener('close', () => sockets.delete(socket), { once: true })
    socket.addEventListener('message', event => {
      // Installed before Engine.IO's listener, so the authentication boundary
      // precedes the production hook's HTTP recovery callback on this page.
      if (typeof event.data !== 'string' || !event.data.startsWith('42')) return
      const [name, value] = JSON.parse(event.data.slice(2))
      if (name === 'authenticated') trace.authentications.push({ at: Date.now(), userId: value?.userId,
        tenantId: value?.tenantId, organizationId: value?.organizationId })
    })
    return socket
  } })
  window.__phase23CloseTransports = () => { for (const socket of sockets) socket.close() }
}

const ownerMatches = (value, owner) => value?.userId === owner.userId && value?.tenantId === owner.tenantId && value?.organizationId === owner.organizationId
const status = page => page.evaluate(() => {
  const state = window.__phase23Auth.getState()
  return { status: state.sessionStatus, version: state.sessionVersion, epoch: state.contextEpoch,
    switching: state.contextSwitching, sessionId: state.sessionId, organizationId: state.profile?.organization.id ?? null,
    oldAborted: Boolean(window.__phase23OldScope?.signal.aborted), contextGone: window.__phase23Context() === null }
})
const querySnapshot = page => page.evaluate(() => window.__phase23Query.getQueryCache().getAll()
  .filter(query => query.queryKey[0] === 'notifications').map(query => ({ key: query.queryKey, status: query.state.status,
    records: query.state.data?.records?.map(row => ({ id: row.id, title: row.title, organizationId: row.organizationId })), count: query.state.data?.count })))
const readyIndicator = page => page.locator('header [aria-label="实时推送已连接"]')
const notificationToast = (page, title) => page.locator('[data-sonner-toast]').filter({ hasText: title })
const wireEvent = (record, name, owner, after = 0) => record.sockets.flatMap(socket => socket.received)
  .filter(event => event.name === name && event.at >= after && ownerMatches(event.value, owner))

/** Exercise production Phase 10 coordination and notification hooks in two real
 * host pages against the caller's real Rust platform, Rust realtime service and
 * PostgreSQL publication path. Credentials remain in memory and are never logged. */
export async function runPhase23NotificationBrowser({ origin, port, platformUrl, notificationUrl, session,
  ownerA, ownerB, publishToOwner, artifactsDir }) {
  assert.equal(origin, `http://127.0.0.1:${port}`)
  for (const url of [platformUrl, notificationUrl]) assert.match(url, /^http:\/\/(?:127\.0\.0\.1|\[::1\]):[0-9]+$/)
  assert(session.accessToken && session.refreshToken, 'A real refreshable platform session is required')
  assert.equal(ownerA.userId, ownerB.userId, 'Both organizations belong to the same shared platform user')
  assert.notEqual(ownerA.organizationId, ownerB.organizationId)
  assert.equal(typeof publishToOwner, 'function')
  let vite, viteExited, browser, context, output = '', spawnError, holdInbox = false, holdContext = false
  const inboxGate = deferred(), contextGate = deferred(), heldInbox = deferred(), heldContext = deferred()
  const errors = [], records = [], checks = []
  const pass = id => { checks.push(id); console.log(`PASS phase23 real browser: ${id}`) }
  const publish = async (owner, title) => publishToOwner(owner, { title, idempotencyKey: randomUUID() })
  const marker = randomUUID().slice(0, 8)
  const titles = { liveA: `Phase23 A live ${marker}`, retiredA: `Phase23 A retired ${marker}`,
    liveB: `Phase23 B live ${marker}`, offlineB: `Phase23 B offline ${marker}` }
  try {
    vite = spawn(process.execPath, [path.join(root, 'frontend/node_modules/vite/bin/vite.js'), '--host', '127.0.0.1', '--port', String(port), '--strictPort'], {
      cwd: path.join(root, 'frontend'), windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'],
      env: { ...commandEnvironment(process.env), BACKEND_PROXY_TARGET: platformUrl, NOTIFICATION_DEV_PROXY_TARGET: notificationUrl },
    })
    viteExited = new Promise(resolve => { vite.once('exit', resolve); vite.once('error', resolve) })
    vite.on('error', error => { spawnError = error })
    for (const stream of [vite.stdout, vite.stderr]) stream.on('data', data => { output = (output + data).slice(-4000) })
    await wait(() => {
      if (spawnError || vite.exitCode !== null) throw new Error('Phase 23 owned Vite startup failed')
      return output.includes('ready in')
    }, 'Real host Vite readiness', 45_000)
    browser = await launchBrowser()
    context = await browser.newContext({ viewport: { width: 1450, height: 1000 } })
    await context.addInitScript(observeNativeTransports, { origin, session })
    await context.route(url => url.origin === origin && url.pathname === '/src/main.tsx', async route => {
      const response = await route.fetch(), body = await response.text()
      assert.match(body, /const queryClient\s*=/)
      await route.fulfill({ response, body: body + `\nimport { authStore as phase23Auth } from '/src/stores/auth.ts';
        import { getNotificationContext as phase23Context, notificationRequestOptions as phase23Options } from '/src/hooks/use-notification-context.ts';
        import { notificationsApi as phase23Notifications } from '/src/api/notifications.ts';
        window.__phase23Auth = phase23Auth; window.__phase23Context = phase23Context; window.__phase23Query = queryClient;
        window.__phase23Options = phase23Options; window.__phase23Notifications = phase23Notifications;
        window.__phase23Successes = [];
        queryClient.getQueryCache().subscribe(event => {
          if (event.query.queryKey[0] === 'notifications' && event.query.state.status === 'success') {
            const data = event.query.state.data;
            window.__phase23Successes.push({ at: Date.now(), key: event.query.queryKey,
              records: data?.records?.map(row => ({id: row.id, title: row.title, organizationId: row.organizationId})) });
          }
        });\n` })
    })
    for (let index = 0; index < 2; index++) {
      const page = await context.newPage(), record = { page: index + 1, sockets: [] }
      page.setDefaultTimeout(25_000)
      records.push(record)
      page.on('pageerror', error => errors.push({ page: index + 1, message: error.message }))
      page.on('websocket', socket => {
        if (new URL(socket.url()).pathname !== '/socket.io/') return
        const entry = { opened: Date.now(), closed: null, received: [], disconnectSent: false }
        record.sockets.push(entry)
        socket.on('close', () => { entry.closed = Date.now() })
        socket.on('framesent', frame => { if (String(frame.payload) === '41') entry.disconnectSent = true })
        socket.on('framereceived', frame => {
          const payload = String(frame.payload)
          if (!payload.startsWith('42')) return
          const [name, value] = JSON.parse(payload.slice(2))
          if (name !== 'authenticated' && name !== 'notification') return
          // Deliberately retain only public owner/event fields, never raw frames.
          entry.received.push({ name, at: Date.now(), value: { contractVersion: value?.contractVersion,
            userId: value?.userId, tenantId: value?.tenantId, organizationId: value?.organizationId,
            ...(name === 'notification' ? { id: value?.id, title: value?.title } : {}) } })
        })
      })
      record.browserPage = page
      await page.goto(origin + '/dashboard', { waitUntil: 'domcontentloaded' })
      await page.waitForFunction(() => window.__phase23Auth && window.__phase23Query)
      await readyIndicator(page).waitFor()
      await wait(async () => (await status(page)).organizationId === ownerA.organizationId, 'Real platform profile is in A')
    }
    const pages = records.map(record => record.browserPage)
    const documentIds = await Promise.all(pages.map(page => page.evaluate(() => window.__phase23Document)))
    const settled = async () => {
      for (const page of pages) await wait(async () => page.evaluate(() =>
        window.__phase23Query.getQueryCache().getAll().filter(query => query.queryKey[0] === 'notifications').every(query => query.state.fetchStatus !== 'fetching') &&
        window.__phase23Transport.requests.every(row => row.completed || row.failed)), 'Previous real HTTP inbox requests reach terminal results')
    }
    const recoveries = async (owner, after) => {
      for (const record of records) {
        await wait(() => wireEvent(record, 'authenticated', owner, after).length > 0, 'Real Rust socket authenticated the expected owner')
        await wait(async () => record.browserPage.evaluate(({ owner, after }) => {
          const requests = window.__phase23Transport.requests
          const authentication = window.__phase23Transport.authentications.findLast(value => value.organizationId === owner.organizationId && value.at >= after)
          return authentication && ['/api/notifications', '/api/notifications/unread-count', '/api/notifications/prefs'].every(path =>
            requests.some(row => row.path === path && row.organizationId === owner.organizationId && row.status === 200 && row.started >= after && row.completed >= after &&
              (path !== '/api/notifications' || row.started >= authentication.at)))
        }, { owner, after }), 'Authenticated socket causes successful real HTTP list/count/preferences recovery')
        await readyIndicator(record.browserPage).waitFor()
      }
    }
    await recoveries(ownerA, 0)
    const shared = await Promise.all(pages.map(status))
    assert.equal(shared[0].sessionId, shared[1].sessionId)
    assert.equal(shared[0].version, shared[1].version)
    pass('two-pages-share-real-platform-session-and-authenticated-http-recovery')

    await publish(ownerA, titles.liveA)
    for (const record of records) {
      await wait(() => wireEvent(record, 'notification', ownerA).some(event => event.value.title === titles.liveA), 'A publication reaches each real socket')
      await notificationToast(record.browserPage, titles.liveA).waitFor()
    }
    pass('persisted-publication-reaches-both-real-host-pages')
    await settled()

    // Hold only the actual server response from A. The next native fetch ignores
    // transport cancellation, so successful fencing cannot be an abort-only pass.
    const a = pages[0]
    await a.route(url => url.origin === origin && url.pathname === '/api/notifications', async route => {
      if (!holdInbox || route.request().method() !== 'GET') return route.continue()
      holdInbox = false
      const response = await route.fetch()
      assert.equal(response.status(), 200, 'Held inbox is an actual successful Rust/PostgreSQL response')
      const body = await response.json()
      assert(body.records.some(row => row.title === titles.liveA), 'The real old inbox contains the persisted A publication')
      heldInbox.resolve()
      await inboxGate.promise
      try { await route.fulfill({ response }) } catch (error) {
        if (!/closed|disposed|cancelled|Invalid Interception/i.test(error.message)) throw error
      }
    })
    holdInbox = true
    await a.evaluate(() => {
      window.__phase23Transport.detachNextInbox = true
      // This promise uses the actual production request/fence without React
      // Query. Cancellation/cache removal alone cannot satisfy its assertion.
      window.__phase23LateResult = null
      void window.__phase23Notifications.list({ filter: 'ALL', page: 1, size: 5 }, window.__phase23Options(window.__phase23Context()))
        .then(() => { window.__phase23LateResult = { resolved: true } }, error => {
          window.__phase23LateResult = { resolved: false, name: error.name }
        })
    })
    await wait(() => heldInbox.resolved, 'A real old inbox response is held before switching')
    for (const page of pages) await page.evaluate(() => { window.__phase23OldScope = window.__phase23Context() })
    await a.route(url => url.origin === origin && url.pathname === '/api/auth/context', async route => {
      if (!holdContext) return route.continue()
      holdContext = false
      assert.equal(route.request().postDataJSON().organizationId, ownerB.organizationId)
      heldContext.resolve()
      await contextGate.promise
      await route.continue()
    })
    holdContext = true
    const targetName = await a.evaluate(orgId => window.__phase23Auth.getState().profile.memberships.find(item => item.organizationId === orgId)?.organizationName, ownerB.organizationId)
    assert(targetName, 'The real profile declares the second direct membership')
    const switchStarted = Date.now()
    await a.getByRole('button', { name: '切换组织上下文', exact: true }).click()
    await a.getByRole('button').filter({ hasText: targetName }).click()
    await wait(() => heldContext.resolved, 'The real UI invokes a held organization context request')
    await wait(async () => (await Promise.all(pages.map(status))).every(state => state.switching && state.status === 'switching' && state.oldAborted && state.contextGone),
      'Shared switching intent aborts the old notification scope in both pages before backend context change')
    for (const record of records) {
      await wait(() => record.sockets.filter(socket => socket.received.some(event => event.name === 'authenticated' && ownerMatches(event.value, ownerA))).every(socket => socket.closed),
        'Both real old organization transports close while the context request is still held')
      assert.equal(await readyIndicator(record.browserPage).count(), 0)
      assert.equal(await notificationToast(record.browserPage, titles.liveA).count(), 0)
    }
    pass('shared-switch-intent-synchronously-retires-both-old-scopes-and-real-transports')
    contextGate.resolve()
    await wait(async () => (await Promise.all(pages.map(status))).every(state => !state.switching && state.status === 'active' && state.organizationId === ownerB.organizationId),
      'Actual Rust context response propagates B through the shared coordinator')
    await recoveries(ownerB, switchStarted)
    const successesBeforeLate = await Promise.all(pages.map(page => page.evaluate(() => window.__phase23Successes.length)))
    inboxGate.resolve()
    await wait(async () => a.evaluate(() => window.__phase23Transport.requests.some(row => row.detached && row.aborted && row.status === 200)),
      'The detached actual A response completes after its signal was aborted')
    await wait(async () => a.evaluate(() => window.__phase23LateResult !== null), 'The actual production late inbox request reaches a terminal result')
    assert.deepEqual(await a.evaluate(() => window.__phase23LateResult), { resolved: false, name: 'AbortError' },
      'The actual production session fence rejects the late 200 independently of React Query')
    for (let index = 0; index < pages.length; index++) {
      const successes = await pages[index].evaluate(start => window.__phase23Successes.slice(start), successesBeforeLate[index])
      assert(!successes.some(row => row.records?.some(item => item.organizationId === ownerA.organizationId)), 'Late A data never becomes successful new-context cache data')
      const snapshot = await querySnapshot(pages[index])
      assert(!snapshot.some(row => row.records?.some(item => item.organizationId === ownerA.organizationId)))
      assert.equal(await notificationToast(pages[index], titles.liveA).count(), 0)
    }
    const switched = await Promise.all(pages.map(status))
    assert.equal(switched[0].sessionId, switched[1].sessionId)
    assert.equal(switched[0].version, switched[1].version)
    pass('real-context-switch-rebinds-both-pages-and-rejects-late-old-http-data')

    const afterSwitch = Date.now()
    await publish(ownerA, titles.retiredA)
    await publish(ownerB, titles.liveB)
    for (const record of records) {
      await wait(() => wireEvent(record, 'notification', ownerB, afterSwitch).some(event => event.value.title === titles.liveB), 'B live publication reaches both fresh sockets')
      await notificationToast(record.browserPage, titles.liveB).waitFor()
    }
    // A positive B delivery fences the finite absence observation; this is not a
    // timing-based assertion that a legitimate event must arrive immediately.
    await delay(450)
    for (const record of records) {
      assert(!wireEvent(record, 'notification', ownerA, afterSwitch).some(event => event.value.title === titles.retiredA))
      assert.equal(await notificationToast(record.browserPage, titles.retiredA).count(), 0)
    }
    pass('post-switch-real-publications-are-isolated-to-current-organization')

    await settled()
    await context.setOffline(true)
    await Promise.all(pages.map(page => page.evaluate(() => window.__phase23CloseTransports())))
    for (const page of pages) await wait(async () => await readyIndicator(page).count() === 0, 'Native transport loss removes connected status')
    await publish(ownerB, titles.offlineB)
    const restored = Date.now()
    await context.setOffline(false)
    await recoveries(ownerB, restored)
    for (const record of records) {
      await wait(async () => (await querySnapshot(record.browserPage)).some(row => row.records?.some(item => item.title === titles.offlineB)),
        'Real HTTP recovery reads the publication persisted during transport loss')
      assert.equal(await notificationToast(record.browserPage, titles.offlineB).count(), 0, 'Historical recovery does not replay Toasts')
      assert(!wireEvent(record, 'notification', ownerA, afterSwitch).some(event => event.value.title === titles.retiredA),
        'No retired-organization event leaks during the complete switch and reconnect observation')
    }
    pass('real-transport-reconnect-recovers-persisted-offline-inbox-without-history-toasts')
    assert.deepEqual(await Promise.all(pages.map(page => page.evaluate(() => window.__phase23Document))), documentIds,
      'The same two documents survive context switching and reconnect without page reload')
    assert.deepEqual(errors, [], 'Both actual host pages remain free of uncaught runtime errors')
    const sourcePaths = ['frontend/src/lib/session-coordinator.ts', 'frontend/src/lib/api.ts', 'frontend/src/stores/auth.ts',
      'frontend/src/hooks/use-notification-context.ts', 'frontend/src/hooks/use-notification-socket.tsx',
      'frontend/src/main.tsx', 'frontend/vite.config.ts', 'tests/phase23-notification-browser.mjs']
    const result = { checks, pages: 2, documentsPreserved: true,
      lateHttp: { upstreamStatus: 200, productionPromise: 'AbortError' },
      browser: { product: browserName, version: await browser.version(), playwright: playwrightVersion },
      sources: sourcePaths.map(file => ({ path: file, sha256: createHash('sha256').update(readFileSync(path.join(root, file))).digest('hex') })),
      observations: await Promise.all(records.map(async record => ({ page: record.page, authenticatedA: wireEvent(record, 'authenticated', ownerA).length,
        authenticatedB: wireEvent(record, 'authenticated', ownerB).length,
        closedTransports: record.sockets.filter(socket => socket.closed).length,
        currentOrganization: ownerB.organizationId,
        http: await record.browserPage.evaluate(() => ['/api/notifications', '/api/notifications/unread-count', '/api/notifications/prefs'].map(path => ({
          path, succeeded: window.__phase23Transport.requests.filter(row => row.path === path && row.status === 200).length,
          aborted: window.__phase23Transport.requests.filter(row => row.path === path && row.aborted).length,
        }))),
      }))),
    }
    if (artifactsDir) {
      mkdirSync(artifactsDir, { recursive: true })
      // The screenshot contains only the owned fixture's public host UI; no
      // storage, authorization headers, session record or token is displayed.
      await a.locator('header button[aria-label$="条未读通知"], header button[aria-label="通知"]').first().click()
      await a.getByText(titles.offlineB, { exact: true }).waitFor()
      await a.screenshot({ path: path.join(artifactsDir, `phase23-notification-${browserName}.png`), fullPage: true })
    }
    console.log('PHASE23_NOTIFICATION_BROWSER ' + JSON.stringify(result))
    return result
  } catch (error) {
    console.error('PHASE23_NOTIFICATION_BROWSER_FAILURE ' + JSON.stringify({ checks, errors,
      pages: await Promise.all(records.map(async record => ({ page: record.page,
        state: record.browserPage.isClosed() ? null : await status(record.browserPage).catch(() => null),
        socketCount: record.sockets.length, closedTransports: record.sockets.filter(socket => socket.closed).length }))) }))
    throw error
  } finally {
    inboxGate.resolve(); contextGate.resolve()
    await context?.close().catch(() => {})
    await browser?.close().catch(() => {})
    if (vite?.pid && vite.exitCode === null && vite.signalCode === null) {
      vite.kill()
      await Promise.race([viteExited, delay(5000)])
      if (vite.exitCode === null && vite.signalCode === null) {
        vite.kill('SIGKILL')
        await Promise.race([viteExited, delay(5000)])
        assert(vite.exitCode !== null || vite.signalCode !== null, 'The owned Vite process must exit during cleanup')
      }
    }
  }
}
