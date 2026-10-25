import assert from 'node:assert/strict'
import test from 'node:test'
import * as runtime from '../src/components/micro/runtime.ts'

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
const application = { id: 'app', name: 'Orders', runtimeMode: 'TRUSTED', entryType: 'HTML',
  entryUrl: '/sub-apps/orders/versions/v1/index.html', deploymentVersion: 'v1', entryVersionId: 'release-v1' }
function context(extra = {}) {
  return { contractVersion: '1.0', contextVersion: 'auth-v1', expiresAt: new Date(61_000).toISOString(),
    appCode: 'orders', application: { ...application }, user: { id: 'user', username: 'u', displayName: 'User' },
    tenant: { id: 'tenant', name: 'Tenant', code: 't' }, organization: { id: 'org', name: 'Org', path: '/org/' },
    organizationScopeIds: ['org'], roles: [], permissions: ['order:read'], menus: [], dataGrants: [], ...extra }
}
const props = signal => { const { application: _entry, ...view } = context(); return ({ ...view, sdkVersion: '3.0.0', contractVersion: '2.0', contextEpoch: 1,
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

test('apps without the optional update lifecycle remount for route, signal or authorization replacement', () => {
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

test('a valid context that expires in transit revokes old authorization and recovers without extending either deadline', async () => {
  const delayed = deferred(), fresh = deferred(), receivedTokens = []
  let calls = 0
  const s = setup({ context: async token => {
    receivedTokens.push(token)
    if (++calls === 2) return delayed.promise
    if (calls === 3) return fresh.promise
    return context()
  } })
  await s.service.refresh(); const first = s.changes.at(-1)
  const expiredContext = context({ expiresAt: new Date(s.time.now() + 500).toISOString() })
  const pending = s.service.refresh()
  await flush(); await s.time.advance(600)
  delayed.resolve(expiredContext); await pending
  assert.ok(first.signal.aborted, 'A delayed expired proof must retire the still-valid old snapshot')
  assert.equal(s.changes.at(-1), null)
  assert.equal(s.changes.filter(Boolean).length, 1, 'The expired response must never be delivered')
  assert.deepEqual(s.errors, [])
  assert.equal(s.issues(), 1)
  await s.time.advance(499); assert.equal(s.issues(), 1)
  await s.time.advance(1); const issuedAt = s.time.now()
  assert.equal(s.issues(), 2)
  assert.deepEqual(receivedTokens, ['token-1', 'token-1', 'token-2'], 'Recovery must discard even an unexpired old application token')
  const freshExpiry = new Date(issuedAt + 10_000).toISOString()
  await s.time.advance(200)
  fresh.resolve(context({ expiresAt: freshExpiry })); await flush()
  const recovered = s.changes.at(-1)
  assert.equal(recovered.token, 'token-2')
  assert.notEqual(recovered.signal, first.signal)
  assert.equal(recovered.signal.aborted, false)
  assert.ok(recovered.contextEpoch > first.contextEpoch)
  assert.equal(recovered.context.expiresAt, freshExpiry)
  assert.equal(recovered.tokenExpiresAt, new Date(issuedAt + 60_000).toISOString(), 'Wire time must consume the issued lease instead of extending it')
  assert.deepEqual(s.errors, [])
  s.service.stop()
})

test('a lease that expires in transit is reissued even when its returned context remains valid', async () => {
  const delayed = deferred(); let issues = 0, contexts = 0
  const s = setup({
    issue: async () => ({ accessToken: `short-token-${++issues}`, expiresIn: 0.5, audience: 'orders' }),
    context: async () => ++contexts === 1 ? delayed.promise : context(),
  })
  const pending = s.service.refresh()
  await flush(); await s.time.advance(501)
  delayed.resolve(context()); await pending
  assert.equal(s.changes.some(Boolean), false)
  assert.equal(s.changes.at(-1), null)
  assert.deepEqual(s.errors, [])
  await s.time.advance(499); assert.equal(issues, 1)
  await s.time.advance(1)
  const recovered = s.changes.at(-1)
  assert.equal(issues, 2); assert.equal(contexts, 2)
  assert.equal(recovered.token, 'short-token-2')
  assert.equal(recovered.tokenExpiresAt, new Date(s.time.now() + 500).toISOString())
  assert.ok(Date.parse(recovered.context.expiresAt) > s.time.now())
  s.service.stop()
})

test('persistent valid but expired contexts exhaust exactly three retries without publishing or retrying forever', async () => {
  const s = setup({ context: async () => context({ expiresAt: new Date(s.time.now()).toISOString() }) })
  await s.service.refresh(); await s.time.advance(3500)
  assert.equal(s.issues(), 4)
  assert.deepEqual(s.recovery.filter(Boolean).map(value => value.delayMs), [500, 1000, 2000])
  assert.equal(s.errors.length, 1)
  assert.equal(s.changes.some(Boolean), false)
  assert.equal(s.time.timers.size, 0)
  await s.time.advance(60_000)
  assert.equal(s.issues(), 4)
})

test('expired wire responses share the total recovery budget and cannot revive after its deadline', async () => {
  const replies = []
  const s = setup({ context: async () => {
    const reply = deferred()
    replies.push({ ...reply, value: context({ expiresAt: new Date(s.time.now() + 1000).toISOString() }) })
    return reply.promise
  } })
  const startedAt = s.time.now(), pending = s.service.refresh()
  await flush()
  for (const [index, retryDelay] of [500, 1000, 2000].entries()) {
    assert.equal(replies.length, index + 1)
    await s.time.advance(4800)
    replies[index].resolve(replies[index].value); await flush()
    assert.equal(s.errors.length, 0)
    assert.equal(s.changes.at(-1), null)
    await s.time.advance(retryDelay)
  }
  assert.equal(s.issues(), 4); assert.equal(replies.length, 4)
  const remaining = 20_000 - (s.time.now() - startedAt)
  assert.ok(remaining > 0 && remaining < 5000, 'The final attempt must inherit the remaining budget')
  await s.time.advance(remaining - 1); assert.equal(s.errors.length, 0)
  await s.time.advance(1); await pending
  assert.equal(s.time.now() - startedAt, 20_000)
  assert.equal(s.errors.length, 1)
  assert.equal(s.time.timers.size, 0)
  replies[3].resolve(context({ expiresAt: new Date(s.time.now() + 60_000).toISOString() })); await flush()
  assert.equal(s.changes.some(Boolean), false, 'A late proof cannot restart an exhausted recovery episode')
  await s.time.advance(60_000); assert.equal(s.issues(), 4)
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

test('transport recovery remains limited to explicit network failures or 503 while permanent errors stay denied', async () => {
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
  const expired = new Date(0).toISOString()
  for (const value of [null, {}, context({ contractVersion: '999.0' }),
    context({ contractVersion: '999.0', expiresAt: expired }), context({ appCode: 'another-app', expiresAt: expired }),
    context({ contextVersion: '', expiresAt: expired }), context({ contextVersion: undefined, expiresAt: expired }),
    context({ contextVersion: 42, expiresAt: expired }), context({ expiresAt: 'not-a-date' })]) {
    const s = setup({ context: async () => value }); await s.service.refresh(); await s.time.advance(20_000)
    assert.equal(s.errors.length, 1); assert.equal(s.issues(), 1)
    assert.equal(s.recovery.some(Boolean), false, 'Malformed proof must not be reclassified as an expired lease')
    assert.equal(s.changes.some(Boolean), false)
    assert.equal(s.time.timers.size, 0)
  }
})

test('entry resolution uses the whole context tuple despite a stale profile deployment', async () => {
  const { resolveMicroEntry, microEntrySignature } = await import('../src/components/micro/entry.ts')
  const profile = { id: 'app', code: 'orders', name: 'Profile name', icon: 'Box', ...application,
    entryUrl: '/stale-profile/index.html', deploymentVersion: 'stale', entryVersionId: 'stale-id' }
  const resolved = resolveMicroEntry(context(), profile)
  assert.equal(resolved.entryUrl, application.entryUrl)
  assert.equal(resolved.deploymentVersion, 'v1')
  assert.equal(resolved.entryVersionId, 'release-v1')
  assert.notEqual(microEntrySignature(resolved), microEntrySignature(profile))
})

test('entry checks reject an activation or rollback between loading resources and building props', async () => {
  const { resolveMicroEntry, assertMicroEntrySnapshot } = await import('../src/components/micro/entry.ts')
  const info = { id: 'app', code: 'orders', name: 'Orders', icon: 'Box' }, captured = resolveMicroEntry(context(), info)
  assertMicroEntrySnapshot(captured, context())
  for (const change of [{ runtimeMode: 'IFRAME' }, { entryType: 'ESM' }, { entryUrl: '/different/index.html' },
    { deploymentVersion: 'v2' }, { entryVersionId: 'release-v2' }]) {
    assert.throws(() => assertMicroEntrySnapshot(captured, context({ application: { ...application, ...change } })),
      error => error.name === 'AbortError')
  }
})

test('archived or unknown entry metadata requires qiankun reconfiguration without a compatibility fallback', async () => {
  const { resolveMicroEntry } = await import('../src/components/micro/entry.ts')
  const identity = { id: 'app', code: 'orders', name: 'Orders', icon: 'Box' }
  for (const legacy of [{ runtimeMode: 'IFRAME' }, { entryType: 'ESM' }, { runtimeMode: 'UNKNOWN' }, { entryType: 'UNKNOWN' }]) {
    assert.throws(() => resolveMicroEntry(context({ application: { ...application, ...legacy } }), identity),
      error => error.code === 'MICRO_ENTRY_RECONFIGURATION_REQUIRED' && error.message.includes('重新配置'))
  }
})

test('invalid entry contracts and a different application binding are explicit permanent failures', async () => {
  const { resolveMicroEntry } = await import('../src/components/micro/entry.ts')
  const info = { id: 'app', code: 'orders', name: 'Orders', icon: 'Box' }
  for (const patch of [{ id: 'another' },
    { entryUrl: '' }, { entryUrl: 'javascript:alert(1)' }, { entryUrl: 'https://user:secret@host.test/app' }]) {
    assert.throws(() => resolveMicroEntry(context({ application: { ...application, ...patch } }), info),
      error => error.code === 'INVALID_APPLICATION_ENTRY' && !error.message.includes('secret'))
  }
})
