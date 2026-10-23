import assert from 'node:assert/strict'
import test from 'node:test'
import { componentFixture } from './helpers/component-fixture.mjs'
import * as runtime from '../src/components/micro/runtime.ts'
import * as entry from '../src/components/micro/entry.ts'
import { normalizedRoutePath } from '../src/lib/route-permissions.ts'

const flush = async () => { for (let i = 0; i < 60; i++) await Promise.resolve() }
const microRouteAccess = componentFixture('src/lib/micro-route-permissions.ts', ['microRouteAccess'], {
  '@/lib/route-permissions': { normalizedRoutePath },
}).exports.microRouteAccess

function node() {
  return { hidden: false, inert: false, children: [],
    replaceChildren(...children) { this.children = children }, remove() { this.removed = true } }
}

/** Run the actual container effects, authorization runtime and lifecycle queue. */
function container(t, { supportsUpdate = true, delayedContext = false } = {}) {
  const location = { pathname: '/micro/orders/detail/7', search: '?status=open', hash: '#summary' }
  const app = { id: 'app', code: 'orders', name: 'Orders', icon: '', entryType: 'HTML', runtimeMode: 'TRUSTED',
    entryUrl: '/micro-proxy/orders/index.html', deploymentVersion: null, entryVersionId: null }
  const context = { contractVersion: '1.0', contextVersion: 'auth-v1', appCode: app.code,
    application: { ...app }, expiresAt: new Date(Date.now() + 120_000).toISOString(),
    user: { id: 'user', username: 'user', displayName: 'User' },
    tenant: { id: 'tenant', code: 'tenant', name: 'Tenant' },
    organization: { id: 'org', name: 'Org', path: '/org/' }, organizationScopeIds: ['org'],
    roles: [], permissions: ['orders:read'], dataGrants: [],
    menus: [{ id: 'detail', type: 'MENU', path: '/micro/orders/detail/:id', parentId: null,
      title: 'Detail', icon: null, sortOrder: 1, visible: true }] }
  const state = { token: 'platform-token', sessionId: 'session', sessionVersion: 1, contextEpoch: 1,
    contextSwitching: false, profile: { ...context, applications: [app] } }
  const useAuthStore = selector => selector(state)
  useAuthStore.getState = () => state
  const calls = [], navigations = [], timers = new Map(), domain = new runtime.TrustedMicroRuntimeDomain()
  let nextTimer = 0, contextReads = 0, issued = 0, releaseContext
  const pendingContext = delayedContext ? new Promise(resolve => { releaseContext = resolve }) : null
  const documentBefore = Object.getOwnPropertyDescriptor(globalThis, 'document')
  Object.defineProperty(globalThis, 'document', { configurable: true, value: { title: 'Platform', createElement: node } })
  const f = componentFixture('src/components/micro/MicroAppContainer.tsx', ['MicroMountZone'], {
    'react-router-dom': { useNavigate: () => (...args) => navigations.push(args), useLocation: () => location },
    '@/stores/auth': { useAuthStore, reconcileSessionSync() {} },
    '@/components/theme-provider': { useTheme: () => ({ resolvedTheme: 'light' }) },
    '@/lib/micro-route-permissions': { microRouteAccess },
    '@/lib/route-permissions': { normalizedRoutePath },
    '@/lib/api': { ApiError: class ApiError extends Error { constructor(message, code, status) { super(message); this.code = code; this.status = status } },
      api: { post: async () => { issued++; return { accessToken: 'app-token', expiresIn: 120, audience: app.code } },
        get: async () => { contextReads++; if (pendingContext) await pendingContext; return context } } },
    './runtime': { ...runtime, trustedMicroRuntimeDomain: domain,
      MicroContextRuntime: class extends runtime.MicroContextRuntime {
        constructor(options) { super({ ...options, setTimer(callback) { const key = ++nextTimer; timers.set(key, callback); return key },
          clearTimer(key) { timers.delete(key) } }) }
      } },
    './entry': entry,
    './qiankun': { createQiankunLifecycle: () => ({ name: app.code,
      mount: async (_container, props) => { calls.push({ kind: 'mount', props }) },
      ...(supportsUpdate ? { update: async props => { calls.push({ kind: 'update', props }) } } : {}),
      unmount: async () => { calls.push({ kind: 'unmount' }) } }) },
    './capabilities': { assertQiankunCapabilities() {} },
    '../../../vendor/micro-app-sdk/protocol.mjs': { SDK_VERSION: '2.0.0', MICRO_PROPS_CONTRACT_VERSION: '2.0' },
  }, { stableCallbacks: true })
  const host = node()
  const render = () => { const view = f.render('MicroMountZone', { entryInfo: app }); view.props.containerRef.current = host; return view }
  const settle = async () => { for (let i = 0; i < 5; i++) { await flush(); render() } await flush() }
  render()
  t.after(async () => {
    releaseContext?.()
    f.unmount()
    await flush()
    assert.equal(timers.size, 0, 'Authorization timers must be released')
    assert.equal(domain.fault, null)
    if (documentBefore) Object.defineProperty(globalThis, 'document', documentBefore)
    else delete globalThis.document
  })
  return { location, calls, navigations, settle, render, releaseContext,
    counts: () => ({ contextReads, issued }),
    context }
}

test('container mounts the complete URL and delivers query-only and hash-only updates without reissuing authorization', async t => {
  const f = container(t)
  await f.settle()
  assert.equal(f.calls[0].kind, 'mount')
  assert.equal(f.calls[0].props.routePath, '/micro/orders/detail/7?status=open#summary')
  const signal = f.calls[0].props.signal
  f.location.search = '?status=closed&next=%2Fmicro%2Fother'
  f.render(); await f.settle()
  assert.equal(f.calls.at(-1).kind, 'update')
  assert.equal(f.calls.at(-1).props.routePath, '/micro/orders/detail/7?status=closed&next=%2Fmicro%2Fother#summary')
  f.location.hash = '#history?not=a-menu'
  f.render(); await f.settle()
  assert.equal(f.calls.at(-1).props.routePath, '/micro/orders/detail/7?status=closed&next=%2Fmicro%2Fother#history?not=a-menu')
  assert.equal(f.calls.at(-1).props.signal, signal, 'Presentation navigation must keep the live authorization lease')
  assert.deepEqual(f.counts(), { contextReads: 1, issued: 1 })
  assert.equal(f.calls.filter(call => call.kind === 'mount').length, 1)
  assert.equal(f.calls.filter(call => call.kind === 'update').length, 2)
  assert.deepEqual(f.navigations, [])
})

test('container checks the latest pathname during delayed authorization and never treats query/hash text as a menu grant', async t => {
  const f = container(t, { delayedContext: true })
  f.location.pathname = '/micro/orders/not-authorized'
  f.location.search = '?return=/micro/orders/detail/7'
  f.location.hash = '#/micro/orders/detail/7'
  f.render()
  f.releaseContext()
  await f.settle()
  assert.deepEqual(f.calls, [])
  assert.equal(f.navigations.at(-1)[0], '/')
})

test('container mounts the latest complete URL if only query/hash changed while authorization was pending', async t => {
  const f = container(t, { delayedContext: true })
  f.location.search = '?status=pending'
  f.location.hash = '#latest'
  f.render(); f.releaseContext(); await f.settle()
  assert.equal(f.calls[0].props.routePath, '/micro/orders/detail/7?status=pending#latest')
  assert.deepEqual(f.counts(), { contextReads: 1, issued: 1 })
})

test('container remount fallback also observes query-only navigation when an existing app has no update export', async t => {
  const f = container(t, { supportsUpdate: false })
  await f.settle()
  const first = f.calls[0].props
  f.location.search = '?status=archived'
  f.render(); await f.settle()
  assert.deepEqual(f.calls.map(call => call.kind), ['mount', 'unmount', 'mount'])
  assert.equal(first.signal.aborted, true)
  assert.equal(f.calls.at(-1).props.routePath, '/micro/orders/detail/7?status=archived#summary')
})
