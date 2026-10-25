import { newUuid } from '../src/lib/uuid.ts'
import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'
import { Script } from 'node:vm'
import ts from '../node_modules/typescript/lib/typescript.js'
import { createMicroServices } from '../src/components/micro/services.ts'
import { MicroLifecycleSession, TrustedMicroRuntimeDomain } from '../src/components/micro/runtime.ts'
import { authorizationContextSignature } from '../vendor/micro-app-sdk/index.mjs'

const source = readFileSync(new URL('../src/components/micro/qiankun.ts', import.meta.url), 'utf8')
const output = ts.transpileModule(source, { compilerOptions: {
  target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS,
} }).outputText
const exports = {}
new Script(`(function(require,exports){${output}\n})`).runInThisContext()(name => {
  if (name === '@/lib/uuid') return { newUuid }
  if (name === './services') return { createMicroServices }
  throw new Error('Unexpected dependency: ' + name)
}, exports)
const { createQiankunLifecycle } = exports
const deferred = () => {
  let resolve, reject
  const promise = new Promise((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}
const tick = async () => { for (let index = 0; index < 20; index++) await Promise.resolve() }
const props = (scope = new AbortController(), epoch = 1) => ({
  sdkVersion: '3.0.0', contractVersion: '2.0', contextVersion: 'auth-' + epoch, contextEpoch: epoch,
  expiresAt: new Date(Date.now() + 60_000).toISOString(), tokenExpiresAt: new Date(Date.now() + 60_000).toISOString(),
  token: 'application-token-' + epoch, appCode: 'orders', basePath: '/micro/orders', routePath: '/micro/orders/list',
  signal: scope.signal, user: { id: 'u' }, tenant: { id: 't' }, organization: { id: 'o' },
  organizationScopeIds: ['o'], roles: [], permissions: [], dataGrants: [], menus: [], theme: 'light',
  onNavigate() {}, onNotify() {},
})

function fixture(options = {}) {
  const previous = Object.getOwnPropertyDescriptor(globalThis, 'window')
  Object.defineProperty(globalThis, 'window', { value: { location: { href: 'https://platform.example.com/micro/orders' } }, configurable: true })
  const scope = new AbortController(), mount = deferred(), unmount = deferred(), calls = [], updates = []
  let unmountCalls = 0, checks = 0
  const parcel = {
    loadPromise: Promise.resolve(null), bootstrapPromise: Promise.resolve(null),
    mountPromise: mount.promise, unmountPromise: unmount.promise,
    unmount() { unmountCalls++; return unmount.promise },
    ...(options.noUpdate ? {} : { update: async value => { updates.push(value) } }),
  }
  const lifecycle = createQiankunLifecycle({
    appCode: 'orders', entryUrl: 'https://app.example.com/orders/index.html?release=1', signal: scope.signal,
    check: () => { checks++; options.check?.() },
    loadMicroApp: (app, config, hooks) => { calls.push({ app, config, hooks }); return parcel },
    ...options,
  })
  const cleanup = async () => {
    mount.resolve(null); unmount.resolve(null)
    try { await lifecycle.unmount() }
    finally {
      if (previous) Object.defineProperty(globalThis, 'window', previous)
      else delete globalThis.window
    }
  }
  return { lifecycle, scope, mount, unmount, calls, updates, parcel, cleanup,
    unmountCalls: () => unmountCalls, checks: () => checks }
}

test('qiankun mounts once and receives the strict platform DTO separately from host capabilities', async () => {
  const s = fixture()
  try {
    const platform = props(), container = {}, mounting = s.lifecycle.mount(container, platform)
    let ready = false
    mounting.then(() => { ready = true })
    await tick()
    assert.equal(s.calls.length, 1)
    assert.equal(ready, false, 'the host must wait for qiankun mountPromise')
    const { app, config, hooks } = s.calls[0]
    assert.equal(app.container, container)
    assert.deepEqual(Object.keys(app.props).sort(), ['platform', 'services'])
    assert.equal(app.props.platform, platform)
    assert.notEqual(authorizationContextSignature(platform), null)
    assert.equal(typeof app.props.services.downloadCsv, 'function')
    const services = app.props.services
    assert.equal(config.prefetch, false)
    assert.equal(config.sandbox.experimentalStyleIsolation, true)
    await hooks.beforeLoad(); await hooks.beforeMount()
    s.mount.resolve(null); await mounting
    assert.equal(ready, true)
    await s.lifecycle.update({ ...platform, theme: 'dark' })
    assert.equal(s.updates[0].platform.theme, 'dark')
    assert.equal(s.updates[0].services, app.props.services)
    const stopping = s.lifecycle.unmount()
    assert.equal(stopping, s.lifecycle.unmount(), 'unmount must be idempotent')
    assert.equal(s.unmountCalls(), 1)
    assert.throws(() => services.downloadCsv({ resource: 'orders', filename: 'orders.csv', content: '1' }), /CSV_DOWNLOAD_DENIED/)
    s.unmount.resolve(null); await stopping
  } finally { await s.cleanup() }
})

test('missing qiankun update remains visible to the host remount fallback', async () => {
  const s = fixture({ noUpdate: true })
  try {
    s.mount.resolve(null)
    await s.lifecycle.mount({}, props())
    assert.equal(s.lifecycle.update, undefined)
  } finally { await s.cleanup() }
})

test('aborted authorization blocks late entry hooks, updates and host capabilities', async () => {
  const s = fixture()
  try {
    const mounting = s.lifecycle.mount({}, props())
    const services = s.calls[0].app.props.services
    s.scope.abort()
    s.mount.resolve(null)
    await assert.rejects(mounting, { name: 'AbortError' })
    await assert.rejects(s.calls[0].hooks.beforeMount(), { name: 'AbortError' })
    await assert.rejects(s.lifecycle.update(props()), { name: 'AbortError' })
    assert.throws(() => services.downloadCsv({}), /CSV_DOWNLOAD_DENIED/)
  } finally { await s.cleanup() }
})

test('static fetch omits credentials, cancels with the active lease and rejects stale response bodies', async () => {
  const requests = [], s = fixture({ fetch: async (url, options) => {
    requests.push({ url, options }); return new Response('entry-source')
  } })
  try {
    const lease = new AbortController(), initial = props(lease)
    s.mount.resolve(null); await s.lifecycle.mount({}, initial)
    const { config } = s.calls[0]
    const response = await config.fetch('https://app.example.com/main.js', {
      credentials: 'include', headers: { Authorization: 'Bearer host-secret', Cookie: 'session=host', Accept: 'text/javascript' },
    })
    assert.equal(requests[0].options.credentials, 'omit')
    assert.equal(requests[0].options.headers.has('Authorization'), false)
    assert.equal(requests[0].options.headers.has('Cookie'), false)
    assert.equal(requests[0].options.headers.get('Accept'), 'text/javascript')
    lease.abort()
    assert.ok(requests[0].options.signal.aborted)
    await assert.rejects(response.text(), { name: 'AbortError' })
    const next = props(new AbortController(), 2)
    await s.lifecycle.update(next)
    assert.equal(await (await config.fetch('https://app.example.com/next.js')).text(), 'entry-source')
    s.scope.abort()
    await assert.rejects(config.fetch('https://app.example.com/late.js'), { name: 'AbortError' })
  } finally { await s.cleanup() }
})

test('an old in-flight request cannot publish content after a context update', async () => {
  const request = deferred(), s = fixture({ fetch: () => request.promise })
  try {
    s.mount.resolve(null); await s.lifecycle.mount({}, props())
    const pending = s.calls[0].config.fetch('https://app.example.com/late.js')
    await s.lifecycle.update(props(new AbortController(), 2))
    request.resolve(new Response('late-old-content'))
    await assert.rejects(pending, { name: 'AbortError' })
  } finally { await s.cleanup() }
})

test('same-authorization route and theme updates preserve an in-flight resource request', async () => {
  const request = deferred(), s = fixture({ fetch: () => request.promise })
  try {
    const initial = props()
    s.mount.resolve(null); await s.lifecycle.mount({}, initial)
    const pending = s.calls[0].config.fetch('https://app.example.com/chunk.js')
    await s.lifecycle.update({ ...initial, theme: 'dark', routePath: '/micro/orders/detail' })
    request.resolve(new Response('current-authorized-content'))
    assert.equal(await (await pending).text(), 'current-authorized-content')
  } finally { await s.cleanup() }
})

test('each execution gets fresh qiankun and HTML cache keys, retaining the deployment query', async () => {
  const first = fixture(), second = fixture()
  try {
    first.mount.resolve(null); second.mount.resolve(null)
    await first.lifecycle.mount({}, props()); await second.lifecycle.mount({}, props())
    const a = first.calls[0].app, b = second.calls[0].app
    assert.notEqual(a.name, b.name)
    assert.notEqual(a.entry, b.entry)
    assert.equal(new URL(a.entry).searchParams.get('release'), '1')
    assert.ok(new URL(a.entry).searchParams.get('__saas_execution'))
  } finally { await second.cleanup(); await first.cleanup() }
})

test('successful remount reuses the execution while cached hooks and fetch use only the replacement lease', async () => {
  const first = fixture({ fetch: async () => new Response('first') })
  let second
  try {
    first.mount.resolve(null)
    const old = props()
    await first.lifecycle.mount({}, old)
    const original = first.calls[0], retainedServices = original.app.props.services
    first.unmount.resolve(null)
    await first.lifecycle.unmount()
    assert.throws(() => original.app.props.platform, { name: 'AbortError' })
    second = fixture({ fetch: async () => new Response('replacement') })
    const replacement = props(new AbortController(), 2)
    second.mount.resolve(null)
    await second.lifecycle.mount({}, replacement)
    assert.equal(second.calls[0].app.name, original.app.name)
    assert.equal(second.calls[0].app.entry, original.app.entry)
    assert.equal(original.app.props.platform, replacement)
    assert.notEqual(original.app.props.services, retainedServices)
    await original.hooks.beforeMount()
    assert.equal(await (await original.config.fetch('https://app.example.com/chunk.js')).text(), 'replacement')
    first.scope.abort()
    assert.equal(await (await original.config.fetch('https://app.example.com/another.js')).text(), 'replacement')
    assert.throws(() => retainedServices.downloadCsv({}), /CSV_DOWNLOAD_DENIED/)
  } finally { await second?.cleanup(); await first.cleanup() }
})

test('entry snapshots are rechecked before code loading and before mount', async () => {
  let valid = true
  const s = fixture({ check: () => { if (!valid) throw new DOMException('entry changed', 'AbortError') } })
  try {
    const mounting = s.lifecycle.mount({}, props())
    valid = false
    await assert.rejects(s.calls[0].hooks.beforeLoad(), { name: 'AbortError' })
    await assert.rejects(s.calls[0].hooks.beforeMount(), { name: 'AbortError' })
    s.mount.resolve(null)
    await assert.rejects(mounting, { name: 'AbortError' })
  } finally { await s.cleanup() }
})

test('the host session waits for qiankun unmount before performing container cleanup', async () => {
  const s = fixture(), events = []
  const session = new MicroLifecycleSession(() => events.push('removed'), undefined, { domain: new TrustedMicroRuntimeDomain() })
  try {
    s.mount.resolve(null)
    await session.mount(async () => s.lifecycle, {}, props())
    const stopping = session.stop()
    await tick()
    assert.equal(s.unmountCalls(), 1)
    assert.deepEqual(events, [])
    s.unmount.resolve(null)
    await stopping
    assert.deepEqual(events, ['removed'])
  } finally { await s.cleanup() }
})

test('failed entry fetch before execution does not poison the host domain by re-unmounting a broken parcel', async () => {
  const s = fixture(), domain = new TrustedMicroRuntimeDomain()
  const session = new MicroLifecycleSession(() => {}, undefined, { domain })
  try {
    const mounting = session.mount(async () => s.lifecycle, {}, props())
    await tick()
    s.mount.reject(new Error('entry fetch failed'))
    await assert.rejects(mounting, /entry fetch failed/)
    await session.stop()
    assert.equal(s.unmountCalls(), 0)
    assert.equal(domain.fault, null)
  } finally { await s.cleanup() }
})

test('single-spa automatic successful unmount after a failed child mount is not invoked twice', async () => {
  const s = fixture(), domain = new TrustedMicroRuntimeDomain()
  const session = new MicroLifecycleSession(() => {}, undefined, { domain })
  try {
    const mounting = session.mount(async () => s.lifecycle, {}, props())
    await tick()
    await s.calls[0].hooks.beforeLoad()
    await s.calls[0].hooks.afterUnmount()
    s.mount.reject(new Error('child mount rejected'))
    await assert.rejects(mounting, /child mount rejected/)
    await session.stop()
    assert.equal(s.unmountCalls(), 0)
    assert.equal(domain.fault, null)
  } finally { await s.cleanup() }
})

test('execution followed by failed mount and unproven qiankun cleanup retains the permanent host fault', async () => {
  const s = fixture(), domain = new TrustedMicroRuntimeDomain()
  const session = new MicroLifecycleSession(() => {}, undefined, { domain })
  try {
    const mounting = session.mount(async () => s.lifecycle, {}, props())
    await tick()
    await s.calls[0].hooks.beforeLoad()
    s.mount.reject(new Error('bootstrap rejected after execution'))
    await assert.rejects(mounting, /bootstrap rejected/)
    const stopping = session.stop()
    s.unmount.reject(new Error('Cannot unmount broken parcel'))
    await assert.rejects(stopping, { code: 'TRUSTED_RUNTIME_FAULT', stage: 'unmount' })
    assert.ok(domain.fault)
  } finally { await s.cleanup().catch(() => {}) }
})
