import assert from 'node:assert/strict'
import test from 'node:test'
import { MicroContextRuntime, MicroLifecycleSession, microPropsSignature } from '../src/components/micro/runtime.ts'
import { createContextClient } from '../vendor/micro-app-sdk/index.mjs'
import { detectBrowserCapabilities, assertBrowserCapabilities } from '../vendor/micro-app-sdk/index.mjs'
import { assertQiankunCapabilities } from '../src/components/micro/capabilities.ts'

const deferred = () => {
  let resolve, reject
  const promise = new Promise((a, b) => { resolve = a; reject = b })
  return { promise, resolve, reject }
}
const tick = async () => { for (let n = 0; n < 10; n++) await Promise.resolve() }
function jsonResponse(value) {
  const body = new ReadableStream({ async start(controller) {
    const result = await value
    try { controller.enqueue(new TextEncoder().encode(JSON.stringify(result))); controller.close() }
    catch { /* The bounded reader may already have cancelled this test stream. */ }
  } })
  return new Response(body, { headers: { 'Content-Type': 'application/json' } })
}
function clock() {
  let time = 1000, next = 0
  const timers = new Map()
  return {
    now: () => time,
    setTimer: (callback, delay) => { const id = ++next; timers.set(id, { at: time + delay, callback }); return id },
    clearTimer: (id) => timers.delete(id),
    timers,
    advance: async (ms) => {
      const target = time + ms
      for (;;) {
        const candidate = [...timers].filter(([, timer]) => timer.at <= target).sort((a, b) => a[1].at - b[1].at)[0]
        if (!candidate) break
        timers.delete(candidate[0]); time = candidate[1].at; candidate[1].callback(); await tick()
      }
      time = target; await tick()
    },
  }
}
function context(time, version = 'v1', org = 'org-a') {
  return { contractVersion: '1.0', appCode: 'orders', contextVersion: version,
    expiresAt: new Date(time + 30_000).toISOString(), application: { id: 'app', name: 'Orders' },
    user: { id: 'user', username: 'user', displayName: 'User' }, tenant: { id: 'tenant', name: 'Tenant', code: 'tenant' },
    organization: { id: org, name: org, path: org }, roles: [], permissions: ['order:read'], dataGrants: [], menus: [] }
}
const lifecycleProps = () => ({ ...context(Date.now()), contractVersion: '2.0', contextEpoch: 1,
  token: 'token', tokenExpiresAt: new Date(Date.now() + 60_000).toISOString(), signal: new AbortController().signal })
function setup(overrides = {}) {
  const time = clock(), changes = [], errors = []
  let issues = 0
  const runtime = new MicroContextRuntime({ appCode: 'orders', ...time,
    issue: async () => ({ accessToken: 'token-' + ++issues, expiresIn: 60, audience: 'orders' }),
    context: async () => context(time.now()), onChange: (value) => changes.push(value),
    onError: (error) => errors.push(error), ...overrides })
  return { time, changes, errors, runtime }
}

test('polls context, renews application token and cancels previous request signals', async () => {
  const { runtime, time, changes, errors } = setup()
  await runtime.refresh()
  const first = changes[0]
  await time.advance(50_000)
  assert.ok(changes.some((value) => value?.token === 'token-2'))
  assert.ok(first.signal.aborted)
  const delivered = changes.filter(Boolean)
  assert.ok(delivered.at(-1).contextEpoch > first.contextEpoch)
  assert.deepEqual(errors, [])
  runtime.stop()
  assert.equal(time.timers.size, 0)
  assert.ok(delivered.at(-1).signal.aborted)
})

test('aborted issuance ignores out-of-order response even if transport ignores AbortSignal', async () => {
  const old = deferred(); let count = 0
  const { runtime, changes } = setup({ issue: async () => ++count === 1 ? old.promise
    : { accessToken: 'new-token', expiresIn: 60, audience: 'orders' } })
  const first = runtime.refresh()
  await runtime.refresh()
  old.resolve({ accessToken: 'old-token', expiresIn: 60, audience: 'orders' })
  await first
  assert.equal(changes.length, 1)
  assert.equal(changes[0].token, 'new-token')
  runtime.stop()
})

test('rapid organization/app replacement never publishes old context after disposal', async () => {
  const late = deferred()
  const a = setup({ context: async () => late.promise })
  const pending = a.runtime.refresh(); await tick(); a.runtime.stop()
  const b = setup(); await b.runtime.refresh()
  late.resolve(context(a.time.now(), 'old', 'old-org')); await pending
  assert.deepEqual(a.changes, [])
  assert.equal(b.changes[0].context.organization.id, 'org-a')
  b.runtime.stop()
})

test('context expiry aborts application work while replacement response is pending', async () => {
  const late = deferred(); let count = 0
  const s = setup()
  s.runtime.stop()
  const changes = []
  const runtime = new MicroContextRuntime({ appCode: 'orders', ...s.time,
    issue: async () => ({ accessToken: 'token', expiresIn: 60, audience: 'orders' }),
    context: async () => ++count === 1 ? context(s.time.now()) : late.promise,
    onChange: (value) => changes.push(value), onError: () => {} })
  await runtime.refresh(); const first = changes[0]
  await s.time.advance(31_000)
  assert.ok(first.signal.aborted)
  assert.equal(changes.at(-1), null)
  runtime.stop(); late.resolve(context(s.time.now())); await tick()
  assert.equal(changes.at(-1), null)
})

test('unavailable authorization fails closed and cancels timers', async () => {
  const failure = new Error('application revoked')
  const { runtime, changes, errors, time } = setup({ context: async () => { throw failure } })
  await runtime.refresh()
  assert.deepEqual(changes, [null])
  assert.deepEqual(errors, [failure])
  assert.equal(time.timers.size, 0)
})

test('unknown context contract versions are rejected before publishing any authorization snapshot', async () => {
  for (const contractVersion of ['2.0', '999.0', '', undefined]) {
    const { runtime, changes, errors, time } = setup({
      context: async () => ({ ...context(1000), contractVersion }),
    })
    await runtime.refresh()
    assert.deepEqual(changes, [null])
    assert.equal(errors.length, 1)
    assert.equal(time.timers.size, 0)
    runtime.stop()
  }
})

test('SDK capabilities require client APIs and reject removed loader modes', () => {
  const client = { fetch() {}, Headers, URL, AbortController, DOMException, TextEncoder, TextDecoder,
    ReadableStream, Response, crypto }
  assert.deepEqual(detectBrowserCapabilities('client', client), { mode: 'client', supported: true, missing: [] })
  for (const mode of ['iframe', 'host']) {
    assert.throws(() => detectBrowserCapabilities(mode, client), TypeError)
  }
  for (const [api, replacement] of [['AbortController', undefined], ['crypto', {}], ['Response', class {}]]) {
    assert.throws(() => assertBrowserCapabilities('client', { ...client, [api]: replacement }),
      (error) => error.code === 'BROWSER_CAPABILITIES_UNSUPPORTED' && error.missing.length > 0)
  }
  assert.equal(detectBrowserCapabilities('client', client).supported, true,
    'A client requires no DOM or message bridge.')
})

test('capability diagnostics contain fixed API names and ignore hostile environment details', () => {
  const environment = new Proxy({}, { get() { throw new Error('application-token-secret') } })
  assert.throws(() => assertBrowserCapabilities('client', environment),
    (error) => error.code === 'BROWSER_CAPABILITIES_UNSUPPORTED' &&
      !error.message.includes('secret') && error.missing.includes('fetch'))
  assert.throws(() => detectBrowserCapabilities('application-token-secret', {}),
    (error) => error instanceof TypeError && !error.message.includes('secret'))
})

test('the qiankun host requires its actual APIs without requiring iframe or MessageChannel support', () => {
  const environment = { fetch() {}, Headers, URL, AbortController, AbortSignal, DOMException, TextEncoder, TextDecoder,
    ReadableStream, Response, Request, crypto, Proxy, DOMParser: class {}, MutationObserver: class {},
    document: { createElement() {} }, Blob }
  assert.deepEqual(assertQiankunCapabilities(environment), { supported: true, mode: 'qiankun' })
  for (const patch of [{ Proxy: undefined }, { DOMParser: undefined }, { Request: undefined },
    { AbortSignal: { timeout: AbortSignal.timeout } }]) {
    assert.throws(() => assertQiankunCapabilities({ ...environment, ...patch }),
      error => error.code === 'BROWSER_CAPABILITIES_UNSUPPORTED' && error.missing.length > 0)
  }
  const hostile = new globalThis.Proxy({}, { get() { throw new Error('private-application-token') } })
  assert.throws(() => assertQiankunCapabilities(hostile),
    error => error.code === 'BROWSER_CAPABILITIES_UNSUPPORTED' && !error.message.includes('private'))
})

test('late asynchronous mount is followed by final unmount and queued updates are dropped', async () => {
  const mounting = deferred(), calls = []
  const session = new MicroLifecycleSession(() => calls.push('cleanup'))
  const lifecycle = { name: 'test', mount: async () => { calls.push('mount'); await mounting.promise; calls.push('mount-end') },
    update: () => calls.push('update'), unmount: () => calls.push('unmount') }
  const mounted = session.mount(async () => lifecycle, {}, {})
  await tick()
  const update = session.update(lifecycleProps()); const disposed = session.stop()
  assert.deepEqual(calls, ['mount'])
  mounting.resolve(); await Promise.all([mounted, update, disposed, session.stop()])
  assert.deepEqual(calls, ['mount', 'mount-end', 'unmount', 'cleanup'])
})

test('unmount waits for asynchronous update and remains idempotent', async () => {
  const updating = deferred(), calls = []
  const session = new MicroLifecycleSession(() => calls.push('cleanup'))
  await session.mount(async () => ({ name: 'test', mount() {}, update: async () => {
    calls.push('update'); await updating.promise; calls.push('update-end') }, unmount: () => calls.push('unmount') }), {}, {})
  const updated = session.update(lifecycleProps()); await tick()
  const stop = session.stop(); updating.resolve(); await Promise.all([updated, stop])
  assert.deepEqual(calls, ['update', 'update-end', 'unmount', 'cleanup'])
})

test('serialized updates discard canceled intermediate contexts behind a slow update', async () => {
  const updating = deferred(), delivered = []
  const session = new MicroLifecycleSession(() => {})
  await session.mount(async () => ({ name: 'test', mount() {}, unmount() {}, update: async (props) => {
    delivered.push(props.contextVersion)
    if (props.contextVersion === 'first') await updating.promise
  } }), {}, lifecycleProps())
  const first = session.update({ ...lifecycleProps(), contextVersion: 'first' }); await tick()
  const canceled = new AbortController()
  const skipped = session.update({ ...lifecycleProps(), contextVersion: 'canceled', signal: canceled.signal })
  const latest = session.update({ ...lifecycleProps(), contextVersion: 'latest' })
  canceled.abort(); updating.resolve(); await Promise.all([first, skipped, latest])
  assert.deepEqual(delivered, ['first', 'latest'])
  await session.stop()
})

test('full prop signature notices same-length menu/permission changes and token renewal', () => {
  const props = { ...context(1000), contractVersion: '2.0', token: 'token-1', tokenExpiresAt: 'expiry',
    contextEpoch: 1, theme: 'light', routePath: '/orders', menus: [{ id: 'one', title: 'First' }] }
  const first = microPropsSignature(props)
  assert.notEqual(first, microPropsSignature({ ...props, menus: [{ id: 'one', title: 'Changed' }] }))
  assert.notEqual(first, microPropsSignature({ ...props, permissions: ['order:write'] }))
  assert.notEqual(first, microPropsSignature({ ...props, token: 'token-2' }))
  assert.notEqual(first, microPropsSignature({ ...props, sdkVersion: '1.0.0' }))
  assert.notEqual(first, microPropsSignature({ ...props, organization: { id: 'org-b' } }))
  assert.notEqual(first, microPropsSignature({ ...props, organizationScopeIds: ['org-a', 'org-child'] }))
  assert.notEqual(microPropsSignature({ ...props, organizationScopeIds: ['org-a'] }),
    microPropsSignature({ ...props, organizationScopeIds: ['org-b'] }))
  assert.notEqual(microPropsSignature({ ...props, organizationScopeIds: ['org-a'] }),
    microPropsSignature({ ...props, organizationScopeIds: null }))
})

test('portable SDK aborts delayed JSON on renewal and sends only the current application token', async () => {
  const body = deferred(), requests = []
  const props = { ...context(Date.now()), contractVersion: '2.0', contextEpoch: 1,
    token: 'old-token', tokenExpiresAt: new Date(Date.now() + 60_000).toISOString(), signal: new AbortController().signal }
  const client = createContextClient(props, async (_, options) => {
    requests.push(options)
    return jsonResponse(requests.length === 1 ? body.promise : { current: true })
  })
  const pending = client.requestJson('/orders')
  const rejected = assert.rejects(pending, (error) => error.name === 'AbortError')
  await tick()
  client.update({ ...props, contextEpoch: 2, token: 'new-token', signal: new AbortController().signal })
  assert.ok(requests[0].signal.aborted)
  body.resolve({ old: true })
  await rejected
  assert.deepEqual(await client.requestJson('/orders'), { current: true })
  assert.equal(requests[1].headers.get('Authorization'), 'Bearer new-token')
  client.dispose(); assert.equal(client.current(), null)
})

test('portable SDK rejects expired contexts and joins caller cancellation with host cancellation', async () => {
  const host = new AbortController(), caller = new AbortController(), body = deferred()
  let requestSignal
  const props = { ...context(Date.now()), contractVersion: '2.0', contextEpoch: 1,
    token: 'token', tokenExpiresAt: new Date(Date.now() + 60_000).toISOString(), signal: host.signal }
  const client = createContextClient(props, async (_, options) => {
    requestSignal = options.signal
    return jsonResponse(body.promise)
  })
  const pending = client.requestJson('/orders', { signal: caller.signal })
  const rejected = assert.rejects(pending, (error) => error.name === 'AbortError')
  await tick()
  caller.abort(); assert.ok(requestSignal.aborted); body.resolve({})
  await rejected
  host.abort(); assert.equal(client.current(), null)
  await assert.rejects(client.requestJson('/orders'), (error) => error.name === 'AbortError')
  client.dispose()
  const expired = createContextClient({ ...props, signal: new AbortController().signal,
    expiresAt: new Date(Date.now() - 1).toISOString() })
  await assert.rejects(expired.requestJson('/orders'), (error) => error.name === 'AbortError')
  expired.dispose()
})
