import assert from 'node:assert/strict'
import test from 'node:test'
import vm from 'node:vm'
import { readFileSync, existsSync } from 'node:fs'
import { MessageChannel } from 'node:worker_threads'
import { createRequire } from 'node:module'
import { createServer } from 'node:http'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { createIframeLifecycle } from '../frontend/src/components/micro/iframe.ts'

const workspace = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const sdkSource = readFileSync(path.join(workspace, 'examples/micro-app-sdk/iframe-bridge.js'), 'utf8')
const deferred = () => { let resolve, reject; const promise = new Promise((a, b) => { resolve = a; reject = b }); return { promise, resolve, reject } }
const turns = async () => { for (let i = 0; i < 10; i++) await new Promise((resolve) => setImmediate(resolve)) }
const until = async (predicate) => {
  const end = Date.now() + 2000
  while (!predicate()) { assert.ok(Date.now() < end, 'protocol event did not arrive'); await new Promise((resolve) => setImmediate(resolve)) }
}
function props(epoch = 1, extra = {}) {
  return { contractVersion: '2.0', contextVersion: 'context-' + epoch, contextEpoch: epoch,
    expiresAt: new Date(Date.now() + 60_000).toISOString(), tokenExpiresAt: new Date(Date.now() + 60_000).toISOString(),
    signal: new AbortController().signal, appCode: 'orders', basePath: '/micro/orders', routePath: '/micro/orders',
    token: 'mock-application-' + epoch, user: { id: 'u', username: 'user', displayName: 'User' },
    tenant: { id: 't', name: 'Tenant', code: 'tenant' }, organization: { id: 'o', name: 'Organization', path: '/o/' },
    roles: [{ id: 'r', name: 'Role', code: 'role' }], permissions: ['orders:read'],
    dataGrants: [{ resource: 'orders', action: 'read', scope: 'SELF', subjectUserId: 'u', tenantId: 't', organizationIds: ['o'] }],
    menus: [{ id: 'm', parentId: null, type: 'MENU', title: 'Orders', path: '/micro/orders', icon: null, sortOrder: 1, visible: true }],
    theme: 'light', onNavigate() {}, onNotify() {}, ...extra }
}
function child(options = {}) {
  const listeners = new Map(), parent = {}, messages = [], timers = new Map()
  let nextTimer = 0, now = Date.now()
  const clockDate = class extends Date { static now() { return now } }
  const window = { parent, fetch: options.fetch || (async () => ({ ok: true, status: 200, json: async () => ({ ok: true }) })),
    addEventListener: (name, listener) => { if (!listeners.has(name)) listeners.set(name, new Set()); listeners.get(name).add(listener) },
    removeEventListener: (name, listener) => listeners.get(name)?.delete(listener) }
  vm.runInNewContext(sdkSource, { window, document: { body: {} }, URL, AbortController, DOMException, Headers, TextEncoder,
    setTimeout: (callback, delay) => { const id = ++nextTimer; timers.set(id, { callback, at: now + delay }); return id },
    clearTimeout: (id) => timers.delete(id), Date: clockDate })
  const calls = []
  const lifecycle = options.lifecycle || { mount: (_, value) => calls.push(['mount', value]), update: (value) => calls.push(['update', value]), unmount: () => calls.push(['unmount']) }
  const bridge = window.DanteMicroIframe.connect({ appCode: 'orders', allowedHostOrigins: ['https://host.test'], lifecycle })
  const dispatch = (event) => { for (const listener of [...(listeners.get('message') || [])]) listener(event) }
  return { window, parent, calls, bridge, messages, listeners, timers, dispatch,
    advance: (ms) => { now += ms; for (const [id, timer] of timers) if (timer.at <= now) { timers.delete(id); timer.callback() } },
    bootstrap: (port, override = {}) => dispatch({ source: parent, origin: 'https://host.test', ports: [port],
      data: { kind: 'DANTE_MICRO_CONNECT', nonce: 'valid-nonce-1234567890', appCode: 'orders', contractVersion: '2.0', contextEpoch: 1 }, ...override }) }
}
function harness(options = {}) {
  const app = child(options), channels = [], frames = [], outbound = [], bootstraps = [], navigations = [], notifications = []
  const observers = []
  class TrackedChannel {
    constructor() {
      const real = new MessageChannel()
      this.port1 = real.port1; this.port2 = real.port2
      this.closed = [false, false]
      for (const [index, port] of [this.port1, this.port2].entries()) {
        const close = port.close.bind(port); port.close = () => { this.closed[index] = true; close() }
      }
      const post = this.port1.postMessage.bind(this.port1)
      this.port1.postMessage = (message) => { outbound.push(structuredClone(message)); post(message) }
      channels.push(this)
    }
  }
  const target = { isConnected: true, nodes: [], contains(frame) { return this.nodes.includes(frame) },
    replaceChildren(...nodes) { this.nodes = nodes; if (options.autoload !== false) queueMicrotask(() => nodes[0]?.onload?.()) } }
  const document = { documentElement: {}, createElement(tag) {
    assert.equal(tag, 'iframe')
    const frame = { attributes: {}, style: {}, setAttribute(key, value) { this.attributes[key] = value },
      contentWindow: { postMessage(data, origin, ports) { bootstraps.push({ data: structuredClone(data), origin, ports });
        app.dispatch({ data, source: app.parent, origin: 'https://host.test', ports }) } },
      remove() { target.nodes = target.nodes.filter((node) => node !== this) } }
    frames.push(frame); return frame
  } }
  const originals = new Map()
  for (const [name, value] of Object.entries({ document, window: { location: { href: 'https://host.test/dashboard', origin: 'https://host.test' } },
    MessageChannel: TrackedChannel, MutationObserver: class { constructor(callback) { this.callback = callback; observers.push(this) } observe() {} disconnect() { this.disconnected = true } } })) {
    originals.set(name, Object.getOwnPropertyDescriptor(globalThis, name)); Object.defineProperty(globalThis, name, { configurable: true, writable: true, value })
  }
  const scope = new AbortController()
  const lifecycle = createIframeLifecycle({ appCode: 'orders', entryUrl: '/apps/orders/', signal: scope.signal,
    handshakeTimeoutMs: 1000, operationTimeoutMs: 1000, ...options.parent })
  const initial = props(1, { onNavigate: (value) => navigations.push(value), onNotify: (...value) => notifications.push(value), ...options.props })
  return { app, frames, channels, outbound, bootstraps, observers, target, scope, lifecycle, initial, navigations, notifications,
    mount: () => lifecycle.mount(target, initial),
    cleanup: async () => { lifecycle.unmount(); app.bridge.dispose(); await turns();
      for (const [name, descriptor] of originals) { if (descriptor) Object.defineProperty(globalThis, name, descriptor); else delete globalThis[name] } } }
}

const csvGrant = () => ({ resource: 'orders', action: 'export', scope: 'TENANT', subjectUserId: 'u', tenantId: 't', organizationIds: [] })
const csvFile = { resource: 'orders', filename: '订单.csv', content: 'id,title\n1,hello\n' }

test('CSV broker validates export permission and data grant without changing iframe isolation', async () => {
  const saved = []
  const h = harness({ props: { permissions: ['orders:export'], dataGrants: [csvGrant()] },
    parent: { saveCsv: (filename, content) => saved.push({ filename, content }) } })
  try {
    await h.mount()
    await h.app.bridge.downloadCsv(csvFile)
    assert.deepEqual(saved, [{ filename: csvFile.filename, content: csvFile.content }])
    assert.equal(h.frames[0].attributes.sandbox, 'allow-scripts allow-forms')
    assert.equal(h.outbound.find((message) => message.kind === 'DANTE_MICRO_DOWNLOAD_RESULT').ok, true)
  } finally { await h.cleanup() }
})

test('CSV broker denies missing operation, missing grant and forged grant identities', async () => {
  for (const value of [
    { permissions: [], dataGrants: [csvGrant()] },
    { permissions: ['orders:export'], dataGrants: [] },
    { permissions: ['orders:export'], dataGrants: [{ ...csvGrant(), tenantId: 'another-tenant' }] },
    { permissions: ['orders:export'], dataGrants: [{ ...csvGrant(), subjectUserId: 'another-user' }] },
    { permissions: ['orders:export'], dataGrants: [{ ...csvGrant(), scope: 'ALL' }] },
  ]) {
    const saved = []
    const h = harness({ props: value, parent: { saveCsv: (...args) => saved.push(args) } })
    try {
      await h.mount()
      await assert.rejects(h.app.bridge.downloadCsv(csvFile), /denied/)
      assert.equal(saved.length, 0)
    } finally { await h.cleanup() }
  }
})

test('CSV host independently rejects unsafe filename, wrong resource and oversized UTF-8 payloads', async () => {
  const saved = []
  const h = harness({ props: { permissions: ['orders:export'], dataGrants: [csvGrant()] },
    parent: { saveCsv: (...args) => saved.push(args) } })
  try {
    await h.mount()
    const nonce = h.bootstraps[0].data.nonce
    let id = 0
    for (const file of [{ ...csvFile, filename: '../orders.csv' }, { ...csvFile, filename: 'orders.csv.exe' },
      { ...csvFile, filename: 'orders\u0000.csv' }, { ...csvFile, resource: 'another' },
      { ...csvFile, content: '汉'.repeat(2 * 1024 * 1024) }]) {
      const requestId = ++id
      h.channels[0].port2.postMessage({ kind: 'DANTE_MICRO_DOWNLOAD', nonce, contextEpoch: 1, requestId, ...file })
      await until(() => h.outbound.some((message) => message.kind === 'DANTE_MICRO_DOWNLOAD_RESULT' && message.requestId === requestId))
      assert.equal(h.outbound.find((message) => message.kind === 'DANTE_MICRO_DOWNLOAD_RESULT' && message.requestId === requestId).ok, false)
    }
    assert.equal(saved.length, 0)
    await assert.rejects(h.app.bridge.downloadCsv({ ...csvFile, content: '汉'.repeat(2 * 1024 * 1024) }), /Invalid CSV/)
  } finally { await h.cleanup() }
})

test('CSV command replay, wrong nonce and old epoch cannot trigger extra host downloads', async () => {
  const saved = []
  const h = harness({ props: { permissions: ['orders:export'], dataGrants: [csvGrant()] },
    parent: { saveCsv: (...args) => saved.push(args) } })
  try {
    await h.mount()
    await h.app.bridge.downloadCsv(csvFile)
    const nonce = h.bootstraps[0].data.nonce
    for (const extra of [{ requestId: 1 }, { requestId: 2, nonce: 'forged' }, { requestId: 3, contextEpoch: 0 }]) {
      h.channels[0].port2.postMessage({ kind: 'DANTE_MICRO_DOWNLOAD', nonce, contextEpoch: 1, requestId: 4, ...csvFile, ...extra })
    }
    await turns()
    assert.equal(saved.length, 1)
    await assert.rejects(h.app.bridge.downloadCsv(csvFile), /denied/)
    assert.equal(saved.length, 1)
  } finally { await h.cleanup() }
})

test('unmount cancels an in-flight CSV promise before host IO and stale callbacks stay unusable', async () => {
  const saved = []
  const h = harness({ props: { permissions: ['orders:export'], dataGrants: [csvGrant()] },
    parent: { saveCsv: (...args) => saved.push(args) } })
  try {
    await h.mount()
    const request = h.app.bridge.downloadCsv(csvFile)
    h.scope.abort()
    await assert.rejects(request, { name: 'AbortError' })
    assert.equal(saved.length, 0)
    await assert.rejects(h.app.bridge.downloadCsv(csvFile), { name: 'AbortError' })
  } finally { await h.cleanup() }
})

test('child bootstrap authenticates browser source, exact configured origin, nonce, application and single port', async () => {
  const app = child(), channel = new MessageChannel(), received = []
  channel.port1.on('message', (data) => received.push(data))
  for (const invalid of [{ source: {} }, { origin: 'null' }, { origin: 'https://host.test.evil' }, { origin: 'https://evil.test', data: { hostOrigin: 'https://host.test' } },
    { data: { kind: 'DANTE_MICRO_CONNECT', nonce: 'short', appCode: 'orders', contractVersion: '2.0', contextEpoch: 1 } },
    { data: { kind: 'DANTE_MICRO_CONNECT', nonce: 'valid-nonce-1234567890', appCode: 'other', contractVersion: '2.0', contextEpoch: 1 } },
    { ports: [] }, { ports: [channel.port2, channel.port2] }]) app.bootstrap(channel.port2, invalid)
  await turns(); assert.deepEqual(received, [])
  app.bootstrap(channel.port2); await until(() => received.length === 1)
  assert.equal(received[0].kind, 'DANTE_MICRO_READY')
  assert.equal(app.listeners.get('message').size, 0)
  app.bridge.dispose(); channel.port1.close(); channel.port2.close()
})

test('SDK rejects wildcard and noncanonical allowlists instead of guessing host origin', () => {
  const app = child()
  for (const allowedHostOrigins of [[], ['*'], ['https://host.test/path'], ['https://host.test/'], ['null']]) {
    assert.throws(() => app.window.DanteMicroIframe.connect({ appCode: 'orders', allowedHostOrigins, lifecycle: { mount() {}, unmount() {} } }))
  }
  app.bridge.dispose()
})

test('host uses opaque sandbox and one transferred channel; props DTO excludes secrets, signals and functions', async () => {
  const value = props(1)
  const h = harness({ props: { deploymentVersion: 'v1', entryVersionId: 'release-v1', platformSession: 'never-send', clientSecret: 'never-send', user: { ...value.user, platformSession: 'never-send' } } })
  try {
    await h.mount()
    assert.equal(h.frames[0].attributes.sandbox, 'allow-scripts allow-forms')
    assert.equal(h.frames[0].referrerPolicy, 'no-referrer')
    assert.equal(h.frames[0].src, 'https://host.test/apps/orders/')
    assert.equal(h.bootstraps.length, 1)
    assert.equal(h.bootstraps[0].origin, '*')
    assert.equal(h.bootstraps[0].ports.length, 1)
    assert.equal('token' in h.bootstraps[0].data, false)
    const dto = h.outbound.find((message) => message.kind === 'DANTE_MICRO_MOUNT').props
    for (const key of ['platformSession', 'clientSecret', 'signal', 'onNavigate', 'onNotify']) assert.equal(key in dto, false)
    assert.equal('platformSession' in dto.user, false)
    assert.equal(dto.token, h.initial.token)
    assert.equal(dto.deploymentVersion, 'v1')
    assert.equal(dto.entryVersionId, 'release-v1')
    assert.equal(dto.dataGrants[0].scope, 'SELF')
    const mounted = h.app.calls[0][1]
    assert.ok(mounted.signal instanceof AbortSignal)
    assert.equal(typeof mounted.onNavigate, 'function')
    assert.equal(typeof mounted.onNotify, 'function')
  } finally { await h.cleanup() }
})

test('parent rejects forged nonce, stale epochs, cross-application navigation and invalid toast messages', async () => {
  const h = harness()
  try {
    await h.mount()
    const nonce = h.bootstraps[0].data.nonce, send = (data) => h.channels[0].port2.postMessage({ nonce, contextEpoch: 1, ...data })
    send({ kind: 'DANTE_MICRO_NOTIFY', nonce: 'forged', message: 'forged', variant: 'info' })
    send({ kind: 'DANTE_MICRO_NOTIFY', contextEpoch: 0, message: 'old', variant: 'info' })
    for (const path of ['/admin', '/micro/orders-other', '/micro/orders%2fsettings', '//evil.test', 'https://evil.test', 'javascript:alert(1)', '../other', '/micro/orders/%2e%2e/other', '/micro/orders/a\\..\\other']) send({ kind: 'DANTE_MICRO_NAVIGATE', path })
    for (const [message, variant] of [['wrong', 'invalid'], ['x'.repeat(1001), 'info'], ['', 'info']]) send({ kind: 'DANTE_MICRO_NOTIFY', message, variant })
    send({ kind: 'DANTE_MICRO_NAVIGATE', path: 'detail?item=1#section' })
    send({ kind: 'DANTE_MICRO_NOTIFY', message: 'accepted', variant: 'success' })
    await until(() => h.notifications.length === 1)
    assert.deepEqual(h.navigations, ['/micro/orders/detail?item=1#section'])
    assert.deepEqual(h.notifications, [['accepted', 'success']])
    h.scope.abort(); send({ kind: 'DANTE_MICRO_NOTIFY', message: 'after stop', variant: 'info' }); await turns()
    assert.equal(h.notifications.length, 1)
  } finally { await h.cleanup() }
})

test('same-epoch route/theme update works and old hydrated callbacks cannot publish', async () => {
  const h = harness()
  try {
    await h.mount(); const old = h.app.calls[0][1]
    await h.lifecycle.update({ ...h.initial, routePath: '/micro/orders/detail', theme: 'dark' })
    const updated = h.app.calls[1][1]
    assert.ok(old.signal.aborted)
    assert.equal(updated.contextEpoch, old.contextEpoch)
    assert.equal(updated.routePath, '/micro/orders/detail')
    old.onNotify('stale', 'error'); old.onNavigate('/micro/orders/stale')
    updated.onNotify('fresh'); await until(() => h.notifications.length === 1)
    assert.deepEqual(h.notifications, [['fresh', 'default']]); assert.deepEqual(h.navigations, [])
  } finally { await h.cleanup() }
})

test('forged acknowledgements cannot settle a lifecycle and stale DTOs cannot replace current child authorization', async () => {
  const gate = deferred(), calls = []
  const h = harness({ lifecycle: { mount: async (_, value) => { calls.push(value); await gate.promise },
    update: (value) => calls.push(value), unmount() {} } })
  try {
    let settled = false
    const mounting = h.mount().then(() => { settled = true })
    await until(() => h.outbound.some((message) => message.kind === 'DANTE_MICRO_MOUNT'))
    const operation = h.outbound.find((message) => message.kind === 'DANTE_MICRO_MOUNT')
    for (const forged of [{ nonce: 'wrong' }, { contextEpoch: 0 }, { requestId: operation.requestId + 1 }]) {
      h.channels[0].port2.postMessage({ kind: 'DANTE_MICRO_ACK', nonce: operation.nonce, contextEpoch: 1, requestId: operation.requestId, ...forged })
    }
    await turns(); assert.equal(settled, false)
    gate.resolve(); await mounting
    await h.lifecycle.update(props(2))
    const oldDTO = operation.props
    h.channels[0].port1.postMessage({ kind: 'DANTE_MICRO_UPDATE', nonce: operation.nonce, contextEpoch: 1, requestId: 999, props: oldDTO })
    h.channels[0].port1.postMessage({ kind: 'DANTE_MICRO_UPDATE', nonce: 'wrong', contextEpoch: 3, requestId: 1000, props: { ...oldDTO, contextEpoch: 3 } })
    await turns()
    assert.equal(h.app.bridge.current().contextEpoch, 2)
    assert.equal(calls.length, 2)
  } finally { gate.resolve(); await h.cleanup() }
})

test('renewal cancels a slow update immediately, serializes replacement and discards late acknowledgements', async () => {
  const gate = deferred(), calls = []
  const h = harness({ lifecycle: { mount: (_, value) => calls.push(['mount', value]),
    update: async (value) => { calls.push(['update', value]); if (value.contextEpoch === 2) await gate.promise }, unmount: () => calls.push(['unmount']) } })
  try {
    await h.mount()
    const controller = new AbortController(), first = props(2, { signal: controller.signal })
    const updating = h.lifecycle.update(first)
    await until(() => calls.length === 2)
    controller.abort()
    await assert.rejects(updating, (error) => error.name === 'AbortError')
    const latest = props(3), replaced = h.lifecycle.update(latest)
    await until(() => calls[1][1].signal.aborted)
    assert.equal(calls.length, 2)
    gate.resolve(); await replaced
    assert.deepEqual(calls.map(([kind, value]) => [kind, value?.contextEpoch]), [['mount', 1], ['update', 2], ['update', 3]])
    assert.equal(h.frames.length, 1)
    const nonce = h.bootstraps[0].data.nonce
    h.channels[0].port1.postMessage({ kind: 'DANTE_MICRO_CONTEXT_ABORT', nonce, contextEpoch: 2 })
    await turns(); assert.equal(h.app.bridge.current().contextEpoch, 3)
  } finally { gate.resolve(); await h.cleanup() }
})

test('application token renewal cancels delayed JSON and request helper omits cookies', async () => {
  const body = deferred(), requests = []
  const h = harness({ fetch: async (url, options) => { requests.push({ url, ...options });
    return { ok: true, status: 200, json: async () => requests.length === 1 ? body.promise : { current: true } } } })
  try {
    await h.mount()
    const requesting = h.app.bridge.requestJson('https://api.test/orders', { credentials: 'include' })
    await until(() => requests.length === 1)
    await h.lifecycle.update(props(2))
    assert.ok(requests[0].signal.aborted)
    body.resolve({ stale: true }); await assert.rejects(requesting, (error) => error.name === 'AbortError')
    assert.deepEqual(await h.app.bridge.requestJson('https://api.test/orders'), { current: true })
    assert.equal(requests[1].headers.get('Authorization'), 'Bearer mock-application-2')
    assert.equal(requests[1].credentials, 'omit')
  } finally { body.resolve({}); await h.cleanup() }
})

test('child expiry blocks new requests and callbacks independently of the parent timer', async () => {
  const h = harness()
  try {
    await h.mount(); const snapshot = h.app.calls[0][1]
    h.app.advance(61_000)
    assert.ok(snapshot.signal.aborted)
    assert.equal(h.app.bridge.current(), null)
    await assert.rejects(h.app.bridge.requestJson('/orders'), (error) => error.name === 'AbortError')
    snapshot.onNotify('expired'); snapshot.onNavigate('/micro/orders/expired'); await turns()
    assert.deepEqual(h.notifications, []); assert.deepEqual(h.navigations, [])
    assert.equal(h.app.timers.size, 0)
  } finally { await h.cleanup() }
})

test('scope cancellation settles a pending handshake, destroys iframe and closes both untransferred ports', async () => {
  const h = harness({ autoload: false })
  try {
    const mounting = h.mount(); await turns(); h.scope.abort()
    await assert.rejects(mounting, (error) => error.name === 'AbortError')
    assert.equal(h.target.nodes.length, 0)
    assert.deepEqual(h.channels[0].closed, [true, true])
    assert.ok(h.observers[0].disconnected)
  } finally { await h.cleanup() }
})

test('props cancellation interrupts handshake and pending first mount without waiting for child lifecycle', async () => {
  for (const autoload of [false, true]) {
    const gate = deferred(), controller = new AbortController()
    const h = harness({ autoload, props: { signal: controller.signal }, lifecycle: { mount: async () => gate.promise, unmount() {} } })
    try {
      const mounting = h.mount(); await until(() => autoload ? h.outbound.some((event) => event.kind === 'DANTE_MICRO_MOUNT') : h.frames.length > 0)
      controller.abort(); await assert.rejects(mounting, (error) => error.name === 'AbortError')
      assert.equal(h.target.nodes.length, 0)
    } finally { gate.resolve(); await h.cleanup() }
  }
})

test('removed node and second document load dispose the capability instead of granting another channel', async () => {
  for (const action of ['detach', 'navigate']) {
    const h = harness()
    try {
      await h.mount()
      if (action === 'detach') { h.target.isConnected = false; h.observers[0].callback() }
      else h.frames[0].onload()
      assert.equal(h.target.nodes.length, 0)
      assert.equal(h.channels[0].closed[0], true)
      assert.equal(h.bootstraps.length, 1)
      await assert.rejects(h.lifecycle.update(props(2)), (error) => error.name === 'AbortError')
    } finally { await h.cleanup() }
  }
})

test('unmount during an infinite update settles promptly and removes frame, timers and listeners', async () => {
  const gate = deferred()
  const h = harness({ lifecycle: { mount() {}, update: () => gate.promise, unmount() {} } })
  try {
    await h.mount(); const updating = h.lifecycle.update(props(2)); await turns()
    h.lifecycle.unmount(); await assert.rejects(updating, (error) => error.name === 'AbortError')
    assert.equal(h.target.nodes.length, 0)
    await until(() => h.app.timers.size === 0)
    assert.equal(h.app.bridge.current(), null)
    assert.equal(h.app.listeners.get('pagehide').size, 0)
    assert.ok(h.observers[0].disconnected)
  } finally { gate.resolve(); await h.cleanup() }
})

test('timeouts and lifecycle errors are diagnostic without reflecting application secrets', async () => {
  for (const failure of ['timeout', 'exception']) {
    const h = harness({ parent: { operationTimeoutMs: 20 }, lifecycle: { mount() {
      if (failure === 'exception') throw new Error('platform-session-secret application-token-secret')
      return new Promise(() => {})
    }, unmount() {} } })
    try { await assert.rejects(h.mount(), (error) => /Iframe application/.test(error.message) && !/secret/.test(error.message)) }
    finally { await h.cleanup() }
  }
})

test('entry URLs reject executable schemes, embedded credentials and token query parameters', async () => {
  for (const entryUrl of ['javascript:alert(1)', 'data:text/html,test', 'https://user:secret@host.test/app', 'https://host.test/app?access_token=secret']) {
    const h = harness({ parent: { entryUrl } })
    try { await assert.rejects(h.mount(), /HTTP\(S\) without credentials/); assert.equal(h.frames.length, 0) }
    finally { await h.cleanup() }
  }
})

// Optional real-browser gate: MICRO_IFRAME_BROWSER=1 node --test tests/micro-iframe-protocol.mjs
test('real same-origin document remains isolated; renewal preserves DOM and only application token reaches CORS API',
  { skip: process.env.MICRO_IFRAME_BROWSER !== '1', timeout: 30_000 }, async () => {
    const require = createRequire(import.meta.url)
    const bundled = path.join(os.homedir(), '.cache/codex-runtimes/codex-primary-runtime/dependencies/node')
    const { chromium } = require(process.env.PLAYWRIGHT_MODULE || path.join(bundled, 'node_modules/playwright'))
    const executablePath = process.env.BROWSER_EXECUTABLE || [chromium.executablePath(),
      path.join(process.env['ProgramFiles(x86)'] || 'C:/Program Files (x86)', 'Microsoft/Edge/Application/msedge.exe'),
      path.join(process.env.ProgramFiles || 'C:/Program Files', 'Microsoft/Edge/Application/msedge.exe')].find(existsSync)
    assert.ok(executablePath, 'Existing Chromium or Edge is required')
    const ts = require(path.join(workspace, 'frontend/node_modules/typescript'))
    const hostModule = ts.transpileModule(readFileSync(path.join(workspace, 'frontend/src/components/micro/iframe.ts'), 'utf8'),
      { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText
    const requests = [], hold = deferred()
    let origin
    const server = createServer(async (request, response) => {
      const url = new URL(request.url, origin)
      if (url.pathname === '/iframe.js') { response.writeHead(200, { 'Content-Type': 'text/javascript' }); response.end(hostModule) }
      else if (url.pathname === '/sdk.js') { response.writeHead(200, { 'Content-Type': 'text/javascript' }); response.end(sdkSource) }
      else if (url.pathname === '/app') { response.writeHead(200, { 'Content-Type': 'text/html' });
        response.end(readFileSync(path.join(workspace, 'tests/fixtures/iframe-app.html'), 'utf8').replaceAll('__HOST_ORIGIN__', origin)) }
      else if (url.pathname === '/api/orders') {
        response.setHeader('Access-Control-Allow-Origin', 'null')
        response.setHeader('Access-Control-Allow-Methods', 'GET, OPTIONS')
        response.setHeader('Access-Control-Allow-Headers', 'Authorization')
        if (request.method === 'OPTIONS') { response.writeHead(204); response.end(); return }
        requests.push({ authorization: request.headers.authorization, cookie: request.headers.cookie, origin: request.headers.origin })
        if (url.searchParams.has('hold')) await hold.promise
        response.writeHead(200, { 'Content-Type': 'application/json' }); response.end(JSON.stringify({ current: !url.searchParams.has('hold') }))
      } else { response.writeHead(200, { 'Content-Type': 'text/html' }); response.end('<!doctype html><div id="mount"></div><script type="module">import {createIframeLifecycle} from "/iframe.js"; window.factory=createIframeLifecycle</script>') }
    })
    await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '::1', resolve) })
    origin = `http://[::1]:${server.address().port}`
    let browser
    try {
      browser = await chromium.launch({ executablePath, headless: true, args: ['--no-proxy-server'] })
      const page = await browser.newPage()
      await page.goto(origin); await page.waitForFunction(() => !!window.factory)
      await page.evaluate(async (dto) => {
        window.__parentMarker = 'protected'; localStorage.setItem('platform-session', 'never-share')
        window.scope = new AbortController(); window.context = new AbortController()
        window.props = { ...dto, signal: window.context.signal, onNavigate: () => {}, onNotify: () => {} }
        window.app = window.factory({ entryUrl: '/app', appCode: 'orders', signal: window.scope.signal })
        await window.app.mount(document.querySelector('#mount'), window.props)
      }, JSON.parse(JSON.stringify(props(1))))
      const frame = page.frameLocator('iframe')
      await frame.locator('#state').waitFor()
      assert.equal(await page.locator('iframe').getAttribute('sandbox'), 'allow-scripts allow-forms')
      assert.equal(await frame.locator('#state').getAttribute('data-parent-blocked'), 'true')
      assert.equal(await frame.locator('#state').getAttribute('data-storage-blocked'), 'true')
      assert.equal(await page.evaluate(() => window.__parentMarker), 'protected')
      await frame.locator('#input').fill('preserved')
      await frame.locator('#slow').click()
      await until(() => requests.length === 1)
      await page.evaluate(async (dto) => { window.context.abort(); window.context = new AbortController()
        window.props = { ...dto, signal: window.context.signal, onNavigate: () => {}, onNotify: () => {} }; await window.app.update(window.props) }, JSON.parse(JSON.stringify(props(2))))
      assert.equal(await frame.locator('#input').inputValue(), 'preserved')
      await frame.locator('#request').click()
      await frame.locator('#state[data-request="current"]').waitFor()
      assert.equal(requests.at(-1).authorization, 'Bearer mock-application-2')
      assert.equal(requests.at(-1).cookie, undefined)
      assert.equal(requests.at(-1).origin, 'null')
      hold.resolve(); await frame.locator('#state[data-old="aborted"]').waitFor()
      await page.evaluate(() => window.scope.abort())
      assert.equal(await page.locator('iframe').count(), 0)
      await page.evaluate(async (dto) => { window.context = new AbortController(); window.scope = new AbortController()
        window.app = window.factory({ entryUrl: '/app?noBridge=1', appCode: 'orders', signal: window.scope.signal })
        window.pending = window.app.mount(document.querySelector('#mount'), { ...dto, signal: window.context.signal, onNavigate() {}, onNotify() {} }).catch((error) => error.name)
      }, JSON.parse(JSON.stringify(props(3))))
      await page.locator('iframe').waitFor(); await page.evaluate(() => window.scope.abort())
      assert.equal(await page.evaluate(() => window.pending), 'AbortError')
      assert.equal(await page.locator('iframe').count(), 0)
    } finally { hold.resolve(); await browser?.close(); await new Promise((resolve) => server.close(resolve)) }
  })
