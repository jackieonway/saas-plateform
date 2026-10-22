import assert from 'node:assert/strict'
import test from 'node:test'
import vm from 'node:vm'
import { readFileSync } from 'node:fs'
import { MessageChannel } from 'node:worker_threads'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { createIframeLifecycle } from '../src/components/micro/iframe.ts'

const workspace = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const sdkSource = readFileSync(path.join(workspace, 'vendor/micro-app-sdk/iframe-bridge.js'), 'utf8')
const deferred = () => { let resolve, reject; const promise = new Promise((a, b) => { resolve = a; reject = b }); return { promise, resolve, reject } }
const turns = async () => { for (let i = 0; i < 10; i++) await new Promise((resolve) => setImmediate(resolve)) }
function jsonResponse(value) {
  const body = new ReadableStream({ async start(controller) {
    const result = await value
    try { controller.enqueue(new TextEncoder().encode(JSON.stringify(result))); controller.close() }
    catch { /* Context cancellation is allowed to close the bounded test reader. */ }
  } })
  return new Response(body, { headers: { 'Content-Type': 'application/json' } })
}
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
  const window = { parent, fetch: options.fetch || (async () => jsonResponse({ ok: true })),
    addEventListener: (name, listener) => { if (!listeners.has(name)) listeners.set(name, new Set()); listeners.get(name).add(listener) },
    removeEventListener: (name, listener) => listeners.get(name)?.delete(listener) }
  Object.assign(window, { Headers, URL, AbortController, DOMException, TextEncoder, TextDecoder,
    ReadableStream, Response, crypto, MessageChannel, postMessage() {} })
  vm.runInNewContext(options.sdkSource ?? sdkSource, { window, document: { body: {} }, URL, AbortController,
    DOMException, Headers, TextEncoder, TextDecoder, Uint8Array, crypto,
    setTimeout: (callback, delay) => { const id = ++nextTimer; timers.set(id, { callback, at: now + delay }); return id },
    clearTimeout: (id) => timers.delete(id), Date: clockDate })
  const calls = []
  const lifecycle = options.lifecycle || { mount: (_, value) => calls.push(['mount', value]), update: (value) => calls.push(['update', value]), unmount: () => calls.push(['unmount']) }
  const bridge = window.MicroIframe.connect({ appCode: 'orders', allowedHostOrigins: ['https://host.test'], lifecycle })
  const dispatch = (event) => { for (const listener of [...(listeners.get('message') || [])]) listener(event) }
  return { window, parent, calls, bridge, messages, listeners, timers, dispatch,
    advance: (ms) => { now += ms; for (const [id, timer] of timers) if (timer.at <= now) { timers.delete(id); timer.callback() } },
    bootstrap: (port, override = {}) => dispatch({ source: parent, origin: 'https://host.test', ports: [port],
      data: { kind: 'MICRO_CONNECT', nonce: 'valid-nonce-1234567890', appCode: 'orders', contractVersion: '2.0', contextEpoch: 1 }, ...override }) }
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
        if (options.readyOverride) ports[0].postMessage({ kind: 'MICRO_READY', nonce: data.nonce,
          appCode: data.appCode, contextEpoch: data.contextEpoch, contractVersion: '2.0', ...options.readyOverride })
        else app.dispatch({ data, source: app.parent, origin: 'https://host.test', ports }) } },
      remove() { target.nodes = target.nodes.filter((node) => node !== this) } }
    frames.push(frame); return frame
  } }
  const originals = new Map()
  class Frame {}
  Frame.prototype.sandbox = {}
  for (const [name, value] of Object.entries({ document, window: { location: { href: 'https://host.test/dashboard', origin: 'https://host.test' } },
    HTMLIFrameElement: Frame, MessageChannel: TrackedChannel,
    MutationObserver: class { constructor(callback) { this.callback = callback; observers.push(this) } observe() {} disconnect() { this.disconnected = true } } })) {
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
    assert.equal(h.outbound.find((message) => message.kind === 'MICRO_DOWNLOAD_RESULT').ok, true)
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
      h.channels[0].port2.postMessage({ kind: 'MICRO_DOWNLOAD', nonce, contextEpoch: 1, requestId, ...file })
      await until(() => h.outbound.some((message) => message.kind === 'MICRO_DOWNLOAD_RESULT' && message.requestId === requestId))
      assert.equal(h.outbound.find((message) => message.kind === 'MICRO_DOWNLOAD_RESULT' && message.requestId === requestId).ok, false)
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
      h.channels[0].port2.postMessage({ kind: 'MICRO_DOWNLOAD', nonce, contextEpoch: 1, requestId: 4, ...csvFile, ...extra })
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
    { data: { kind: 'MICRO_CONNECT', nonce: 'short', appCode: 'orders', contractVersion: '2.0', contextEpoch: 1 } },
    { data: { kind: 'MICRO_CONNECT', nonce: 'valid-nonce-1234567890', appCode: 'other', contractVersion: '2.0', contextEpoch: 1 } },
    { data: { kind: 'MICRO_CONNECT', nonce: 'valid-nonce-1234567890', appCode: 'orders', contractVersion: '999.0', contextEpoch: 1 } },
    { ports: [] }, { ports: [channel.port2, channel.port2] }]) app.bootstrap(channel.port2, invalid)
  await turns(); assert.deepEqual(received, [])
  app.bootstrap(channel.port2); await until(() => received.length === 1)
  assert.equal(received[0].kind, 'MICRO_READY')
  assert.equal(app.listeners.get('message').size, 0)
  app.bridge.dispose(); channel.port1.close(); channel.port2.close()
})

test('SDK rejects wildcard and noncanonical allowlists instead of guessing host origin', () => {
  const app = child()
  for (const allowedHostOrigins of [[], ['*'], ['https://host.test/path'], ['https://host.test/'], ['null']]) {
    assert.throws(() => app.window.MicroIframe.connect({ appCode: 'orders', allowedHostOrigins, lifecycle: { mount() {}, unmount() {} } }))
  }
  app.bridge.dispose()
})

test('SDK refuses an unknown host SDK version with a fixed diagnostic and no authorization delivery', async () => {
  const app = child(), channel = new MessageChannel(), received = []
  channel.port1.on('message', (message) => received.push(message))
  try {
    app.bootstrap(channel.port2, { data: { kind: 'MICRO_CONNECT', nonce: 'valid-nonce-1234567890',
      appCode: 'orders', contractVersion: '2.0', contextEpoch: 1, sdkVersion: '999.0.0-token-secret' } })
    await until(() => received.length > 0)
    assert.deepEqual(received, [{ kind: 'MICRO_ERROR', nonce: 'valid-nonce-1234567890',
      contextEpoch: 1, code: 'UNSUPPORTED_SDK_VERSION' }])
    assert.equal(app.bridge.current(), null)
    assert.deepEqual(app.calls, [])
    assert.equal(JSON.stringify(received).includes('secret'), false)
  } finally { app.bridge.dispose(); channel.port1.close(); channel.port2.close() }
})

test('host still mounts the exact recorded legacy 2.0 bridge without an SDK version field', async () => {
  const legacy = readFileSync(path.join(workspace, 'vendor/micro-app-sdk/compat/legacy-iframe-2.0.js'), 'utf8')
  const h = harness({ sdkSource: legacy, props: { sdkVersion: '1.0.0' } })
  try {
    await h.mount()
    assert.equal(h.app.bridge.sdkVersion, undefined)
    assert.equal(h.app.calls[0][0], 'mount')
    assert.equal(h.app.calls[0][1].contractVersion, '2.0')
    await h.lifecycle.update(props(2, { sdkVersion: '1.0.0' }))
    assert.equal(h.app.calls.at(-1)[1].contextEpoch, 2)
  } finally { await h.cleanup() }
})

test('unknown props versions and missing isolation APIs are rejected before creating an iframe', async () => {
  for (const contractVersion of ['1.0', '3.0', '999.0']) {
    const h = harness({ props: { contractVersion } })
    try {
      await assert.rejects(h.mount(), /authorization context is invalid/)
      assert.equal(h.frames.length, 0)
      assert.equal(h.bootstraps.length, 0)
    } finally { await h.cleanup() }
  }
  for (const sdkVersion of ['999.0.0', null, undefined]) {
    const h = harness({ props: { sdkVersion } })
    try {
      await assert.rejects(h.mount(), /SDK version is unsupported/)
      assert.equal(h.frames.length, 0)
      assert.equal(h.bootstraps.length, 0)
    } finally { await h.cleanup() }
  }
  const h = harness()
  const original = globalThis.MessageChannel
  try {
    globalThis.MessageChannel = undefined
    await assert.rejects(h.mount(), (error) => error.code === 'BROWSER_CAPABILITIES_UNSUPPORTED' &&
      error.missing.includes('MessageChannel'))
    assert.equal(h.frames.length, 0)
    assert.equal(h.bootstraps.length, 0)
  } finally { globalThis.MessageChannel = original; await h.cleanup() }
})

test('host rejects declared unknown SDK and wire versions without waiting for a handshake timeout', async () => {
  for (const [readyOverride, message] of [[{ sdkVersion: '999.0.0' }, /SDK version is unsupported/],
    [{ contractVersion: '999.0' }, /protocol version is unsupported/],
    [{ kind: 'MICRO_ERROR', code: 'UNSUPPORTED_SDK_VERSION' }, /SDK version is unsupported/]]) {
    const h = harness({ readyOverride })
    try {
      await assert.rejects(h.mount(), message)
      assert.equal(h.outbound.some((entry) => entry.kind === 'MICRO_MOUNT'), false)
      assert.equal(h.target.nodes.length, 0)
      assert.equal(h.bootstraps[0].data.sdkVersion, '1.0.0')
    } finally { await h.cleanup() }
  }
})

test('host uses opaque sandbox and one transferred channel; props DTO excludes secrets, signals and functions', async () => {
  const value = props(1)
  const h = harness({ props: { sdkVersion: '1.0.0', deploymentVersion: 'v1', entryVersionId: 'release-v1', platformSession: 'never-send', clientSecret: 'never-send', user: { ...value.user, platformSession: 'never-send' } } })
  try {
    await h.mount()
    assert.equal(h.frames[0].attributes.sandbox, 'allow-scripts allow-forms')
    assert.equal(h.frames[0].referrerPolicy, 'no-referrer')
    assert.equal(h.frames[0].src, 'https://host.test/apps/orders/')
    assert.equal(h.bootstraps.length, 1)
    assert.equal(h.bootstraps[0].origin, '*')
    assert.equal(h.bootstraps[0].ports.length, 1)
    assert.equal('token' in h.bootstraps[0].data, false)
    const dto = h.outbound.find((message) => message.kind === 'MICRO_MOUNT').props
    for (const key of ['platformSession', 'clientSecret', 'signal', 'onNavigate', 'onNotify']) assert.equal(key in dto, false)
    assert.equal('platformSession' in dto.user, false)
    assert.equal(dto.token, h.initial.token)
    assert.equal(dto.sdkVersion, '1.0.0')
    assert.equal(dto.deploymentVersion, 'v1')
    assert.equal(dto.entryVersionId, 'release-v1')
    assert.equal(dto.dataGrants[0].scope, 'SELF')
    const mounted = h.app.calls[0][1]
    assert.ok(mounted.signal instanceof AbortSignal)
    assert.equal(typeof mounted.onNavigate, 'function')
    assert.equal(typeof mounted.onNotify, 'function')
  } finally { await h.cleanup() }
})

test('organization scope cap is forwarded on mount and renewal with safe legacy fallback', async () => {
  for (const [cap, expected] of [[undefined, ['o']], [['o', 'child-o'], ['o', 'child-o']], [null, null]]) {
    const h = harness({ props: { organizationScopeIds: cap } })
    try {
      await h.mount()
      assert.deepEqual(h.outbound.find((message) => message.kind === 'MICRO_MOUNT').props.organizationScopeIds, expected)
      assert.deepEqual(h.app.calls[0][1].organizationScopeIds, expected)
      await h.lifecycle.update(props(2, { organizationScopeIds: ['o'] }))
      const update = h.outbound.find((message) => message.kind === 'MICRO_UPDATE').props
      assert.deepEqual(update.organizationScopeIds, ['o'])
      assert.deepEqual(h.app.calls.at(-1)[1].organizationScopeIds, ['o'])
    } finally { await h.cleanup() }
  }
})

test('parent rejects forged nonce, stale epochs, cross-application navigation and invalid toast messages', async () => {
  const h = harness()
  try {
    await h.mount()
    const nonce = h.bootstraps[0].data.nonce, send = (data) => h.channels[0].port2.postMessage({ nonce, contextEpoch: 1, ...data })
    send({ kind: 'MICRO_NOTIFY', nonce: 'forged', message: 'forged', variant: 'info' })
    send({ kind: 'MICRO_NOTIFY', contextEpoch: 0, message: 'old', variant: 'info' })
    for (const path of ['/admin', '/micro/orders-other', '/micro/orders%2fsettings', '//evil.test', 'https://evil.test', 'javascript:alert(1)', '../other', '/micro/orders/%2e%2e/other', '/micro/orders/a\\..\\other']) send({ kind: 'MICRO_NAVIGATE', path })
    for (const [message, variant] of [['wrong', 'invalid'], ['x'.repeat(1001), 'info'], ['', 'info']]) send({ kind: 'MICRO_NOTIFY', message, variant })
    send({ kind: 'MICRO_NAVIGATE', path: 'detail?item=1#section' })
    send({ kind: 'MICRO_NOTIFY', message: 'accepted', variant: 'success' })
    await until(() => h.notifications.length === 1)
    assert.deepEqual(h.navigations, ['/micro/orders/detail?item=1#section'])
    assert.deepEqual(h.notifications, [['accepted', 'success']])
    h.scope.abort(); send({ kind: 'MICRO_NOTIFY', message: 'after stop', variant: 'info' }); await turns()
    assert.equal(h.notifications.length, 1)
  } finally { await h.cleanup() }
})

test('same-authorization route/theme updates preserve signals and callbacks until real authorization replacement', async () => {
  const h = harness()
  try {
    await h.mount(); const old = h.app.calls[0][1]
    await h.lifecycle.update({ ...h.initial, routePath: '/micro/orders/detail', theme: 'dark' })
    const updated = h.app.calls[1][1]
    assert.equal(old.signal.aborted, false)
    assert.equal(updated.signal, old.signal)
    assert.equal(updated.contextEpoch, old.contextEpoch)
    assert.equal(updated.routePath, '/micro/orders/detail')
    old.onNotify('same authorization', 'info')
    updated.onNotify('fresh'); await until(() => h.notifications.length === 2)
    assert.deepEqual(h.notifications, [['same authorization', 'info'], ['fresh', 'default']])
    await h.lifecycle.update(props(2))
    assert.equal(old.signal.aborted, true)
    old.onNotify('stale', 'error'); old.onNavigate('/micro/orders/stale'); await turns()
    assert.equal(h.notifications.length, 2); assert.deepEqual(h.navigations, [])
  } finally { await h.cleanup() }
})

test('same-authorization presentation updates keep an in-flight JSON response and CSV acknowledgement usable', async () => {
  const body = deferred(), h = harness({ props: { permissions: ['orders:export'], dataGrants: [csvGrant()] },
    fetch: async () => jsonResponse(body.promise), parent: { saveCsv() {} } })
  try {
    await h.mount()
    const before = h.app.bridge.current(), port = h.channels[0].port1, post = port.postMessage.bind(port)
    let acknowledge
    port.postMessage = data => { if (data.kind === 'MICRO_DOWNLOAD_RESULT') acknowledge = data; else post(data) }
    const reading = h.app.bridge.requestJson('/orders').then(value => ({ value }), error => ({ error }))
    const downloading = h.app.bridge.downloadCsv(csvFile).then(() => ({ ok: true }), error => ({ error }))
    await until(() => acknowledge)
    await h.lifecycle.update({ ...h.initial, theme: 'dark', routePath: '/micro/orders/detail' })
    assert.equal(before.signal.aborted, false)
    assert.equal(h.app.bridge.current().signal, before.signal)
    body.resolve({ sameAuthorization: true }); post(acknowledge)
    const read = await reading
    assert.equal(read.error, undefined); assert.equal(read.value?.sameAuthorization, true)
    assert.deepEqual(await downloading, { ok: true })
  } finally { await h.cleanup() }
})

test('same-epoch changed grants or credentials replace the bridge lifetime and reject old in-flight responses', async () => {
  for (const change of [{ dataGrants: [] }, { token: 'renewed-same-epoch' },
    { expiresAt: new Date(Date.now() + 120000).toISOString() }]) {
    const body = deferred(), h = harness({ fetch: async () => jsonResponse(body.promise) })
    try {
      await h.mount(); const before = h.app.bridge.current()
      const reading = h.app.bridge.requestJson('/orders').then(value => ({ value }), error => ({ error }))
      await h.lifecycle.update({ ...h.initial, ...change })
      assert.equal(before.signal.aborted, true)
      body.resolve({ oldAuthorization: true })
      assert.equal((await reading).error?.name, 'AbortError')
    } finally { await h.cleanup() }
  }
})

test('an iframe lifecycle without update retains pure-theme work and keeps serial fallback for route or authorization changes', async () => {
  const calls = [], h = harness({ lifecycle: { mount(_, value) { calls.push(['mount', value]) }, unmount() { calls.push(['unmount']) } } })
  try {
    await h.mount(); const initial = calls[0][1]
    await h.lifecycle.update({ ...h.initial, theme: 'dark' })
    assert.deepEqual(calls.map(call => call[0]), ['mount'])
    assert.equal(initial.signal.aborted, false)
    await h.lifecycle.update({ ...h.initial, theme: 'dark', routePath: '/micro/orders/detail' })
    assert.deepEqual(calls.map(call => call[0]), ['mount', 'unmount', 'mount'])
    assert.equal(initial.signal.aborted, true)
    const routed = calls.at(-1)[1]
    await h.lifecycle.update(props(2))
    assert.deepEqual(calls.map(call => call[0]), ['mount', 'unmount', 'mount', 'unmount', 'mount'])
    assert.equal(routed.signal.aborted, true)
  } finally { await h.cleanup() }
})

test('forged acknowledgements cannot settle a lifecycle and stale DTOs cannot replace current child authorization', async () => {
  const gate = deferred(), calls = []
  const h = harness({ lifecycle: { mount: async (_, value) => { calls.push(value); await gate.promise },
    update: (value) => calls.push(value), unmount() {} } })
  try {
    let settled = false
    const mounting = h.mount().then(() => { settled = true })
    await until(() => h.outbound.some((message) => message.kind === 'MICRO_MOUNT'))
    const operation = h.outbound.find((message) => message.kind === 'MICRO_MOUNT')
    for (const forged of [{ nonce: 'wrong' }, { contextEpoch: 0 }, { requestId: operation.requestId + 1 }]) {
      h.channels[0].port2.postMessage({ kind: 'MICRO_ACK', nonce: operation.nonce, contextEpoch: 1, requestId: operation.requestId, ...forged })
    }
    await turns(); assert.equal(settled, false)
    gate.resolve(); await mounting
    await h.lifecycle.update(props(2))
    const oldDTO = operation.props
    h.channels[0].port1.postMessage({ kind: 'MICRO_UPDATE', nonce: operation.nonce, contextEpoch: 1, requestId: 999, props: oldDTO })
    h.channels[0].port1.postMessage({ kind: 'MICRO_UPDATE', nonce: 'wrong', contextEpoch: 3, requestId: 1000, props: { ...oldDTO, contextEpoch: 3 } })
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
    h.channels[0].port1.postMessage({ kind: 'MICRO_CONTEXT_ABORT', nonce, contextEpoch: 2 })
    await turns(); assert.equal(h.app.bridge.current().contextEpoch, 3)
  } finally { gate.resolve(); await h.cleanup() }
})

test('application token renewal cancels delayed JSON and request helper omits cookies', async () => {
  const body = deferred(), requests = []
  const h = harness({ fetch: async (url, options) => { requests.push({ url, ...options });
    return jsonResponse(requests.length === 1 ? body.promise : { current: true }) } })
  try {
    await h.mount()
    const requesting = h.app.bridge.requestJson('https://api.test/orders', { credentials: 'include' })
    const rejected = assert.rejects(requesting, (error) => error.name === 'AbortError')
    await until(() => requests.length === 1)
    await h.lifecycle.update(props(2))
    assert.ok(requests[0].signal.aborted)
    body.resolve({ stale: true }); await rejected
    assert.deepEqual(structuredClone(await h.app.bridge.requestJson('https://api.test/orders')), { current: true })
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
      const mounting = h.mount(); await until(() => autoload ? h.outbound.some((event) => event.kind === 'MICRO_MOUNT') : h.frames.length > 0)
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

test('unexpected document and channel failures after mounting notify host recovery exactly once', async () => {
  for (const action of ['navigate', 'messageerror', 'loaderror']) {
    const errors = []
    const h = harness({ parent: { onError: error => errors.push(error) } })
    try {
      await h.mount()
      if (action === 'navigate') h.frames[0].onload()
      else if (action === 'messageerror') h.channels[0].port1.onmessageerror()
      else h.frames[0].onerror()
      assert.equal(errors.length, 1)
      assert.match(errors[0].message, /Iframe application/)
      assert.equal(h.target.nodes.length, 0)
      assert.equal(h.channels[0].closed[0], true)
      h.lifecycle.unmount()
      assert.equal(errors.length, 1)
    } finally { await h.cleanup() }
  }
})

test('normal unmount and authorization retirement do not report a runtime failure', async () => {
  for (const action of ['unmount', 'scope', 'context']) {
    const errors = [], controller = new AbortController()
    const h = harness({ parent: { onError: error => errors.push(error) }, props: { signal: controller.signal } })
    try {
      await h.mount()
      if (action === 'unmount') h.lifecycle.unmount()
      else if (action === 'scope') h.scope.abort()
      else controller.abort()
      await turns()
      assert.deepEqual(errors, [])
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
