import assert from 'node:assert/strict'
import test from 'node:test'
import * as runtime from '../frontend/src/components/micro/runtime.ts'

const flush = async () => { for (let i = 0; i < 40; i++) await Promise.resolve() }
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no }); return { promise, resolve, reject } }
function clock() {
  let time = 1000, next = 0
  const timers = new Map()
  return { now: () => time, timers,
    setTimer(callback, delay) { const id = ++next; timers.set(id, { at: time + delay, callback }); return id },
    clearTimer(id) { timers.delete(id) },
    async advance(ms) {
      const end = time + ms; await flush()
      for (;;) {
        const candidate = [...timers].filter(([, value]) => value.at <= end).sort((a, b) => a[1].at - b[1].at)[0]
        if (!candidate) break
        timers.delete(candidate[0]); time = candidate[1].at; candidate[1].callback(); await flush()
      }
      time = end; await flush()
    } }
}
const application = { id: 'app', name: 'Orders', runtimeMode: 'IFRAME', entryType: 'HTML',
  entryUrl: '/sub-apps/orders/versions/v1/index.html', deploymentVersion: 'v1', entryVersionId: 'release-v1' }
function context(extra = {}) {
  return { contractVersion: '1.0', contextVersion: 'auth-v1', expiresAt: new Date(61_000).toISOString(),
    appCode: 'orders', application: { ...application }, user: { id: 'user', username: 'u', displayName: 'User' },
    tenant: { id: 'tenant', name: 'Tenant', code: 't' }, organization: { id: 'org', name: 'Org', path: '/org/' },
    organizationScopeIds: ['org'], roles: [], permissions: ['order:read'], menus: [], dataGrants: [], ...extra }
}
const props = signal => { const { application: _entry, ...view } = context(); return ({ ...view, sdkVersion: '1.0.0', contractVersion: '2.0', contextEpoch: 1,
  token: 'app-token', tokenExpiresAt: new Date(Date.now() + 60_000).toISOString(),
  expiresAt: new Date(Date.now() + 60_000).toISOString(), signal, theme: 'light', basePath: '/micro/orders', routePath: '/micro/orders',
  deploymentVersion: 'v1', entryVersionId: 'release-v1', onNavigate() {}, onNotify() {} }) }
function setup(overrides = {}) {
  const time = clock(), changes = [], errors = [], recovery = []
  let issueCount = 0
  const service = new runtime.MicroContextRuntime({ appCode: 'orders', ...time,
    issue: async () => ({ accessToken: 'token-' + ++issueCount, expiresIn: 60, audience: 'orders' }),
    context: async () => context(), onChange: value => changes.push(value), onError: error => errors.push(error),
    onRecovery: value => recovery.push(value), ...overrides })
  return { time, changes, errors, recovery, service, issues: () => issueCount }
}
const transient = (status = 503, code = 'SERVER_ERROR') => Object.assign(new Error('service unavailable'), { status, code })

test('trusted display updates reuse the same combined authorization signal while new parent controllers replace it', async () => {
  const captured = [], domain = new runtime.TrustedMicroRuntimeDomain(), scope = new AbortController()
  const session = new runtime.MicroLifecycleSession(() => {}, undefined, { domain })
  const initial = props(scope.signal)
  await session.mount(async () => ({ name: 'orders', mount: (_, value) => captured.push(value),
    update: value => captured.push(value), unmount() {} }), {}, initial)
  await session.update({ ...initial, theme: 'dark' })
  await session.update({ ...initial, routePath: '/micro/orders/detail' })
  assert.equal(captured[1].signal, captured[0].signal)
  assert.equal(captured[2].signal, captured[0].signal)
  assert.equal(captured[0].signal.aborted, false)
  const nextScope = new AbortController()
  await session.update({ ...initial, signal: nextScope.signal })
  assert.notEqual(captured[3].signal, captured[0].signal)
  scope.abort(); assert.ok(captured[0].signal.aborted)
  assert.equal(captured[3].signal.aborted, false)
  await session.stop(); assert.ok(captured[3].signal.aborted)
})

test('an exactly unchanged authorization response preserves its epoch and lifetime without extending expiry', async () => {
  const s = setup()
  await s.service.refresh(); const first = s.changes.at(-1)
  await s.service.refresh(); const second = s.changes.at(-1)
  assert.equal(second.signal, first.signal)
  assert.equal(second.contextEpoch, first.contextEpoch)
  assert.equal(second.context.expiresAt, first.context.expiresAt)
  assert.equal(s.issues(), 1)
  s.service.stop()
})

test('legacy apps without update keep pure display changes but remount for route, signal or authorization replacement', () => {
  const scope = new AbortController(), initial = props(scope.signal), accepted = runtime.microPropsDelivery(initial)
  assert.equal(runtime.requiresMicroRemount(accepted, { ...initial, theme: 'dark' }), false)
  assert.equal(runtime.requiresMicroRemount(accepted, { ...initial, routePath: '/micro/orders/detail' }), true)
  assert.equal(runtime.requiresMicroRemount(accepted, { ...initial, signal: new AbortController().signal }), true)
  assert.equal(runtime.requiresMicroRemount(accepted, { ...initial, permissions: [] }), true)
  initial.permissions = []
  assert.equal(runtime.requiresMicroRemount(accepted, initial), true, 'The accepted authorization identity is cached before mutation.')
})

test('authorization, membership, complete entry tuple and deadline changes still cancel the prior snapshot', async () => {
  for (const patch of [
    { contextVersion: 'auth-v2' }, { user: { id: 'different-user' } }, { organizationScopeIds: [] },
    { permissions: [] }, { dataGrants: [{ resource: 'order', action: 'read', scope: 'SELF' }] },
    { expiresAt: new Date(60_000).toISOString() }, { menus: [{ id: 'changed' }] },
    { application: { ...application, entryUrl: '/sub-apps/orders/versions/v2/index.html', deploymentVersion: 'v2', entryVersionId: 'release-v2' } },
  ]) {
    let count = 0
    const s = setup({ context: async () => count++ ? context(patch) : context() })
    await s.service.refresh(); const first = s.changes.at(-1)
    await s.service.refresh(); const second = s.changes.at(-1)
    assert.ok(first.signal.aborted)
    assert.ok(second.contextEpoch > first.contextEpoch)
    assert.notEqual(first.signal, second.signal)
    s.service.stop()
  }
})

test('a temporary 503 immediately removes old authorization and recovers with a freshly issued token', async () => {
  let calls = 0
  const s = setup({ context: async () => { if (++calls === 2) throw transient(); return context() } })
  await s.service.refresh(); const first = s.changes.at(-1)
  await s.service.refresh()
  assert.ok(first.signal.aborted)
  assert.equal(s.changes.at(-1), null)
  assert.deepEqual(s.errors, [])
  await s.time.advance(499); assert.equal(calls, 2)
  await s.time.advance(1)
  assert.equal(s.changes.at(-1).token, 'token-2')
  assert.ok(s.changes.at(-1).contextEpoch > first.contextEpoch)
  assert.equal(s.issues(), 2)
  s.service.stop()
})

test('persistent transient failures stop after three bounded retries instead of continuing indefinitely', async () => {
  const s = setup({ context: async () => { throw transient() } })
  await s.service.refresh()
  await s.time.advance(3500)
  assert.equal(s.issues(), 4)
  assert.equal(s.errors.length, 1)
  assert.equal(s.changes.at(-1), null)
  assert.equal(s.time.timers.size, 0)
  await s.time.advance(60_000); assert.equal(s.issues(), 4)
})

test('only an explicit network failure or 503 is retried; permanent and protocol failures remain denied', async () => {
  for (const failure of [transient(401), transient(403), transient(404), transient(500), transient(0, 'INVALID_CONTEXT'),
    new TypeError('not a transport classification'), new Error('contract mismatch'), Object.assign(new Error('mode mismatch'), { code: 'MODE_MISMATCH' })]) {
    const s = setup({ context: async () => { throw failure } })
    await s.service.refresh(); await s.time.advance(20_000)
    assert.equal(s.issues(), 1)
    assert.equal(s.errors.length, 1)
    assert.equal(s.time.timers.size, 0)
  }
  let count = 0
  const s = setup({ context: async () => { if (!count++) throw transient(0, 'NETWORK_ERROR'); return context() } })
  await s.service.refresh(); await s.time.advance(500)
  assert.equal(s.issues(), 2); assert.ok(s.changes.at(-1)); s.service.stop()
})

test('an ignored transport abort cannot exceed the attempt and total recovery budgets or publish late issuance', async () => {
  const time = clock(), late = deferred(), changes = [], errors = []
  let issues = 0
  const service = new runtime.MicroContextRuntime({ appCode: 'orders', ...time,
    issue: async () => { issues++; return late.promise }, context: async () => context(),
    onChange: value => changes.push(value), onError: error => errors.push(error) })
  const pending = service.refresh()
  await time.advance(20_000); await pending
  assert.equal(issues, 4)
  assert.equal(errors.length, 1)
  assert.equal(time.timers.size, 0)
  late.resolve({ accessToken: 'late-token', expiresIn: 60, audience: 'orders' }); await flush()
  assert.equal(changes.some(Boolean), false)
})

test('stop during recovery cancels its scheduled retry and no obsolete success is delivered', async () => {
  const s = setup({ context: async () => { throw transient() } })
  await s.service.refresh(); s.service.stop(); await s.time.advance(20_000)
  assert.equal(s.issues(), 1)
  assert.equal(s.changes.some(Boolean), false)
  assert.equal(s.time.timers.size, 0)
})

test('stop settles a pending public refresh even when the transport ignores its abort signal', async () => {
  const late = deferred(), s = setup({ issue: async () => late.promise })
  let settled = false
  const refresh = s.service.refresh().then(() => { settled = true })
  await flush(); s.service.stop(); await flush()
  assert.equal(settled, true)
  assert.equal(s.time.timers.size, 0)
  late.reject(new Error('late transport rejection')); await refresh; await flush()
  assert.equal(s.changes.some(Boolean), false)
})

test('expiry removes the old snapshot while a new context request remains pending and late success is ignored after stop', async () => {
  const late = deferred(); let count = 0
  const s = setup({ context: async () => count++ ? late.promise : context({ expiresAt: new Date(2000).toISOString() }) })
  await s.service.refresh(); const first = s.changes.at(-1)
  await s.time.advance(1001)
  assert.ok(first.signal.aborted)
  assert.equal(s.changes.at(-1), null)
  s.service.stop()
  late.resolve(context()); await flush()
  assert.equal(s.changes.at(-1), null)
  assert.equal(s.time.timers.size, 0)
})

test('a trusted final-unmount fault cancels scheduled authorization recovery without bypassing the page latch', async () => {
  const time = clock(), domain = new runtime.TrustedMicroRuntimeDomain(), never = deferred()
  const session = new runtime.MicroLifecycleSession(() => {}, undefined,
    { domain, ...time, deadlinesMs: { unmount: 10 } })
  await session.mount(async () => ({ name: 'orders', mount() {}, unmount: () => never.promise }), {}, props(new AbortController().signal))
  let service, calls = 0
  const s = setup({ ...time, context: async () => { if (++calls === 2) throw transient(); return context() },
    onChange: value => { if (!value) void session.stop().catch(() => {}) } })
  service = s.service
  const unsubscribe = domain.subscribe(() => service.stop())
  await service.refresh(); await service.refresh()
  await time.advance(10)
  assert.equal(domain.fault.stage, 'unmount')
  await time.advance(20_000)
  assert.equal(s.issues(), 1)
  assert.equal(time.timers.size, 0)
  never.reject(new Error('late trusted rejection')); await flush(); unsubscribe()
})

test('cached authorization identity detects in-place changes and unknown context fields are never reused', async () => {
  const shared = context(), s = setup({ context: async () => shared })
  await s.service.refresh(); const first = s.changes.at(-1)
  shared.permissions = []
  await s.service.refresh(); assert.ok(first.signal.aborted); s.service.stop()
  const unknown = setup({ context: async () => context({ futureAuthorization: { allowed: true } }) })
  await unknown.service.refresh(); const old = unknown.changes.at(-1)
  await unknown.service.refresh(); assert.ok(old.signal.aborted); unknown.service.stop()
})

test('malformed issuance and context responses fail as protocol errors without a network retry', async () => {
  for (const issue of [async () => null, async () => ({}), async () => ({ accessToken: 'x', expiresIn: NaN, audience: 'orders' })]) {
    const s = setup({ issue }); await s.service.refresh(); await s.time.advance(20_000)
    assert.equal(s.errors.length, 1); assert.equal(s.time.timers.size, 0)
  }
  for (const value of [null, {}, context({ contractVersion: '999.0' })]) {
    const s = setup({ context: async () => value }); await s.service.refresh(); await s.time.advance(20_000)
    assert.equal(s.errors.length, 1); assert.equal(s.issues(), 1)
  }
})

test('entry resolution uses the whole context tuple despite a stale profile deployment', async () => {
  const { resolveMicroEntry, microEntrySignature } = await import('../frontend/src/components/micro/entry.ts')
  const profile = { id: 'app', code: 'orders', name: 'Profile name', icon: 'Box', ...application,
    entryUrl: '/stale-profile/index.html', deploymentVersion: 'stale', entryVersionId: 'stale-id' }
  const resolved = resolveMicroEntry(context(), profile)
  assert.equal(resolved.entryUrl, application.entryUrl)
  assert.equal(resolved.deploymentVersion, 'v1')
  assert.equal(resolved.entryVersionId, 'release-v1')
  assert.notEqual(microEntrySignature(resolved), microEntrySignature(profile))
})

test('entry checks reject an activation or rollback between loading resources and building props', async () => {
  const { resolveMicroEntry, assertMicroEntrySnapshot } = await import('../frontend/src/components/micro/entry.ts')
  const info = { id: 'app', code: 'orders', name: 'Orders', icon: 'Box' }, captured = resolveMicroEntry(context(), info)
  assertMicroEntrySnapshot(captured, context())
  for (const change of [{ runtimeMode: 'TRUSTED' }, { entryType: 'ESM' }, { entryUrl: '/different/index.html' },
    { deploymentVersion: 'v2' }, { entryVersionId: 'release-v2' }]) {
    assert.throws(() => assertMicroEntrySnapshot(captured, context({ application: { ...application, ...change } })),
      error => error.name === 'AbortError')
  }
})

test('invalid entry contracts and a different application binding are explicit permanent failures', async () => {
  const { resolveMicroEntry } = await import('../frontend/src/components/micro/entry.ts')
  const info = { id: 'app', code: 'orders', name: 'Orders', icon: 'Box' }
  for (const patch of [{ id: 'another' }, { runtimeMode: 'UNKNOWN' }, { entryType: 'UNKNOWN' },
    { entryUrl: '' }, { entryUrl: 'javascript:alert(1)' }, { entryUrl: 'https://user:secret@host.test/app' }]) {
    assert.throws(() => resolveMicroEntry(context({ application: { ...application, ...patch } }), info),
      error => error.code === 'INVALID_APPLICATION_ENTRY' && !error.message.includes('secret'))
  }
})
