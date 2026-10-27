import assert from 'node:assert/strict'
import test from 'node:test'
import fs from 'node:fs'
import vm from 'node:vm'
import path from 'node:path'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const require = createRequire(path.join(root, 'package.json'))
const ts = require('typescript')
const id = value => `019d0000-0000-7000-8000-${value.toString(16).padStart(12, '0')}`
const csrf = 'c'.repeat(64)
const account = { id: id(1), username: 'owned-bff-test', realName: 'Owned fixture', avatar: '' }
const identity = (patch = {}) => ({ userId: account.id, tenantId: id(2), organizationId: id(3), membershipId: id(4),
  memberType: 'EMPLOYEE', identityVersion: '1', accountVersion: '0', ...patch })
const otherIdentity = identity({ organizationId: id(5), membershipId: id(6) })
const member = proof => ({ userId: proof.userId, tenantId: proof.tenantId, organizationId: proof.organizationId,
  membershipId: proof.membershipId, memberType: proof.memberType, identityVersion: proof.identityVersion,
  organizationName: 'Owned organization', organizationPath: '/Owned', tenantName: 'Owned tenant',
  status: 'ENABLED', consoleSelectable: true, selectionDisabledReason: null })
const instant = Date.parse('2030-01-01T00:00:00Z')
const session = (patch = {}) => ({ sessionId: id(7), version: '1', account: { ...account },
  memberships: [member(identity()), member(otherIdentity)], context: identity(),
  expiresAt: new Date(instant + 3_600_000).toISOString(), csrfToken: csrf, ...patch })
const context = (proof = identity(), version = '1', patch = {}) => ({ sessionId: id(7), version,
  accessToken: `owned-access-${version}`, expiresIn: 300,
  user: { ...account, email: '', phone: '', gender: 'UNKNOWN', isSuperAdmin: false },
  tenant: { id: proof.tenantId, name: 'Owned tenant', code: 'owned' },
  organization: { id: proof.organizationId, name: 'Owned organization', path: '/Owned' },
  membershipId: proof.membershipId, memberType: proof.memberType, identityVersion: proof.identityVersion,
  accountVersion: proof.accountVersion, ...patch })
const json = (status, body) => new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } })
const deferred = () => { let resolve; const promise = new Promise(done => { resolve = done }); return { promise, resolve } }
const settle = () => new Promise(resolve => setImmediate(resolve))

function fixture(handler, options = {}) {
  const calls = [], notices = [], removed = [], timers = new Map(), cache = new Map()
  let clock = instant, nextTimer = 0, callbacks, unsubscribed = false
  const sandbox = vm.createContext({ Response, URL, URLSearchParams, DOMException, AbortController, AbortSignal, Promise,
    setTimeout, clearTimeout })
  function load(relative) {
    if (cache.has(relative)) return cache.get(relative)
    const source = fs.readFileSync(path.join(root, relative), 'utf8')
    const compiled = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } }).outputText
    const exports = {}; cache.set(relative, exports)
    new vm.Script(`(function(require,exports){${compiled}\n})`, { filename: relative }).runInContext(sandbox)(name => {
      assert(name.startsWith('@/'), 'BFF runtime imports must stay local and never load a legacy credential coordinator')
      assert.notEqual(name, '@/lib/session-coordinator', 'Session types must be erased from the BFF runtime')
      return load(`src/${name.slice(2)}.ts`)
    }, exports)
    return exports
  }
  const { createBffSessionCoordinator } = load('src/lib/bff-session.ts')
  const environment = {
    navigate: options.navigate,
    now: () => clock,
    fetch: async (url, config) => { calls.push({ url, config }); return handler(url, config) },
    setTimeout: (callback, ms) => { const timer = ++nextTimer; timers.set(timer, { callback, due: clock + ms }); return timer },
    clearTimeout: timer => timers.delete(timer),
    storage: {
      removeItem: key => { removed.push(key); if (options.storageUnavailable) throw new Error('owned unavailable storage') },
      getItem: () => { throw new Error('BFF must not read stored credentials') },
      setItem: () => { throw new Error('BFF must not persist credentials') },
    },
    notify: notice => notices.push(notice),
    listen: (invalidated, focused, suspended) => { callbacks = { invalidated, focused, suspended }; return () => { unsubscribed = true } },
  }
  const coordinator = createBffSessionCoordinator(environment)
  return { coordinator, calls, notices, removed, timers,
    callbacks: () => callbacks, unsubscribed: () => unsubscribed,
    fence: () => { const record = coordinator.current().record; return { sessionId: record?.sessionId ?? null, version: record?.version ?? -1, contextKey: record?.contextKey ?? null } },
    advance: milliseconds => {
      clock += milliseconds
      for (const [timer, scheduled] of [...timers]) if (scheduled.due <= clock && timers.delete(timer)) scheduled.callback()
    },
    setTime: milliseconds => { clock = instant + milliseconds },
  }
}

test('centralized switch freezes the current context and navigates without posting a member choice', async t => {
  let destination
  const f = fixture(async (url, config) => {
    if (url === '/auth/session') return json(200, session({ centralizedContextSelection: true }))
    if (url === '/auth/context-token') return json(200, context())
    assert.equal(url, '/auth/context-switch')
    assert.equal(config.headers['X-CSRF-Token'], csrf)
    assert.deepEqual(JSON.parse(config.body), { expectedVersion: '1', returnPath: '/dashboard' })
    return json(200, { redirectTo: 'https://auth.example.test/oidc/authorize?context_request=owned' })
  }, { navigate: url => { destination = url } })
  t.after(() => f.coordinator.dispose())
  await f.coordinator.initialize()
  await f.coordinator.beginContextSwitch(f.fence(), '/dashboard')
  assert.equal(destination, 'https://auth.example.test/oidc/authorize?context_request=owned')
  assert.equal(f.coordinator.current().status, 'switching')
  assert.equal(f.coordinator.current().record?.token, null)
  assert.equal(f.calls.filter(call => call.url === '/auth/context-switch').length, 1)
})

test('centralized switch rejects an untrusted navigation target', async t => {
  let navigated = false
  const f = fixture(async url => {
    if (url === '/auth/session') return json(200, session({ centralizedContextSelection: true }))
    if (url === '/auth/context-token') return json(200, context())
    return json(200, { redirectTo: 'https://auth.example.test/other-path' })
  }, { navigate: () => { navigated = true } })
  t.after(() => f.coordinator.dispose())
  await f.coordinator.initialize()
  await assert.rejects(f.coordinator.beginContextSwitch(f.fence(), '/dashboard'))
  assert.equal(navigated, false)
  assert.equal(f.coordinator.current().record?.token, null)
})

test('Cookie recovery uses no stored credentials or Web Locks and exposes no CSRF in observations', async t => {
  const f = fixture(async (url, config) => {
    assert.equal(config.credentials, 'same-origin'); assert.equal(config.cache, 'no-store'); assert.equal(config.redirect, 'error')
    assert.equal(config.headers.Authorization, undefined)
    if (url === '/auth/session') { assert.equal(config.method, 'GET'); return json(200, session()) }
    assert.equal(url, '/auth/context-token'); assert.equal(config.method, 'POST')
    assert.equal(config.headers['X-CSRF-Token'], csrf)
    assert.deepEqual(JSON.parse(config.body), { expectedVersion: '1' })
    return json(200, context())
  }, { storageUnavailable: true })
  t.after(() => f.coordinator.dispose())
  await f.coordinator.initialize()
  const current = f.coordinator.current()
  assert.equal(current.status, 'active'); assert.equal(current.record.token, 'owned-access-1')
  assert.equal('refreshToken' in current.record, false); assert.equal(current.record.sessionId, `${id(7)}:1`)
  assert.equal(current.tokenExpiresAt, instant + 300_000)
  assert.equal(JSON.stringify(current).includes(csrf), false)
  assert.equal(Object.hasOwn(current.session, 'csrfToken'), false)
  assert.deepEqual([...f.removed].sort(), ['platform-session-v2', 'platform-session-v1', 'platform-token', 'platform-refresh-token', 'platform-remember-username', 'platform-current-app'].sort())
  assert.equal(f.notices.length, 0)
})

test('memberless authentication remains authenticated without inventing a context or requesting a bearer', async t => {
  const f = fixture(async url => { assert.equal(url, '/auth/session'); return json(200, session({ memberships: [], context: null })) })
  t.after(() => f.coordinator.dispose())
  await f.coordinator.initialize()
  assert.equal(f.coordinator.current().status, 'active')
  assert.equal(f.coordinator.current().session.account.id, account.id)
  assert.equal(f.coordinator.current().record.token, null); assert.equal(f.coordinator.current().record.identity, null)
  assert.equal(f.calls.length, 1)
})

test('parallel reconciliations coalesce the full GET and bridge recovery', { timeout: 10_000 }, async t => {
  const started = deferred(), released = deferred()
  const f = fixture(async url => {
    if (url === '/auth/session') { started.resolve(); await released.promise; return json(200, session()) }
    return json(200, context())
  })
  t.after(() => f.coordinator.dispose())
  const first = f.coordinator.initialize(), second = f.coordinator.reconcile()
  await started.promise; released.resolve()
  await Promise.all([first, second])
  assert.equal(f.calls.filter(call => call.url === '/auth/session').length, 1)
  assert.equal(f.calls.filter(call => call.url === '/auth/context-token').length, 1)
})

test('parallel renewals share one bridge and preserve the logical context fence', async t => {
  let bridges = 0
  const f = fixture(async url => url === '/auth/session' ? json(200, session()) : json(200, context(identity(), '1', { accessToken: `owned-rotation-${++bridges}` })))
  t.after(() => f.coordinator.dispose())
  await f.coordinator.initialize()
  const before = f.fence()
  const tokens = await Promise.all([f.coordinator.renew(before), f.coordinator.renew(before), f.coordinator.renew(before)])
  assert.deepEqual(tokens, ['owned-rotation-2', 'owned-rotation-2', 'owned-rotation-2'])
  assert.equal(bridges, 2); assert.equal(f.fence().sessionId, before.sessionId); assert.equal(f.fence().contextKey, before.contextKey)
  assert(f.fence().version > before.version)
})

test('short bridge tokens renew before expiry without changing the context namespace', async t => {
  let bridges = 0
  const f = fixture(async url => url === '/auth/session' ? json(200, session()) : json(200, context(identity(), '1', { accessToken: `owned-timer-${++bridges}` })))
  t.after(() => f.coordinator.dispose())
  await f.coordinator.initialize()
  const before = f.fence()
  f.advance(270_000); await settle()
  assert.equal(bridges, 2); assert.equal(f.fence().sessionId, before.sessionId); assert.equal(f.fence().contextKey, before.contextKey)
  assert.equal(f.coordinator.current().tokenExpiresAt, instant + 570_000)
})

for (const expiresIn of [0, 301, 1.5]) test(`bridge expiry ${expiresIn} fails closed instead of accepting a long or malformed bearer`, async t => {
  const f = fixture(async url => url === '/auth/session' ? json(200, session()) : json(200, context(identity(), '1', { expiresIn })))
  t.after(() => f.coordinator.dispose())
  await assert.rejects(f.coordinator.initialize(), error => error.code === 'SESSION_UNKNOWN')
  assert.equal(f.coordinator.current().status, 'unknown'); assert.equal(f.coordinator.current().record.token, null)
})

test('503 and expired leases pause bearer use and never call a legacy endpoint', async t => {
  let unavailable = false
  const f = fixture(async url => unavailable ? json(503, {}) : url === '/auth/session' ? json(200, session()) : json(200, context()))
  t.after(() => f.coordinator.dispose())
  await f.coordinator.initialize()
  unavailable = true
  await assert.rejects(f.coordinator.renew(f.fence()), error => error.status === 503)
  assert.equal(f.coordinator.current().status, 'unknown'); assert.equal(f.coordinator.current().record.token, null)
  await assert.rejects(f.coordinator.reconcile(), error => error.status === 503)
  assert(f.calls.every(call => ['/auth/session', '/auth/context-token'].includes(call.url)))
})

test('401 authority makes the page anonymous without retaining a usable bearer', async t => {
  const f = fixture(async () => json(401, { ignored: 'not copied into diagnostics' }))
  t.after(() => f.coordinator.dispose())
  await f.coordinator.initialize()
  assert.equal(f.coordinator.current().status, 'anonymous')
  assert.equal(f.coordinator.current().session, null); assert.equal(f.coordinator.current().record.token, null)
})

test('selected employee identity and exact decimal CAS survive A to B to A without an old fence becoming usable', async t => {
  const initialVersion = '9007199254740993'
  let current = session({ version: initialVersion })
  const f = fixture(async (url, config) => {
    if (url === '/auth/session') return json(200, current)
    const body = JSON.parse(config.body)
    assert.equal(body.expectedVersion, current.version)
    if (body.membershipId) {
      const selected = body.membershipId === identity().membershipId ? identity() : otherIdentity
      current = { ...current, version: (BigInt(current.version) + 1n).toString(), context: selected }
    }
    return json(200, context(current.context, current.version))
  })
  t.after(() => f.coordinator.dispose())
  await f.coordinator.initialize()
  const oldFence = f.fence()
  await f.coordinator.selectContext(otherIdentity.organizationId, otherIdentity.membershipId, oldFence)
  await f.coordinator.selectContext(identity().organizationId, identity().membershipId, f.fence())
  assert.equal(f.coordinator.current().session.version, '9007199254740995')
  const count = f.calls.length
  await assert.rejects(f.coordinator.renew(oldFence), error => error.name === 'AbortError')
  assert.equal(f.calls.length, count)
  assert.equal(f.notices.length, 2)
  for (const notice of f.notices) assert.deepEqual(Object.keys(notice).sort(), ['invalid', 'revision', 'sessionId'])
  assert.equal(JSON.stringify(f.notices).includes('owned-access'), false)
  assert.equal(JSON.stringify(f.notices).includes(csrf), false)
})

test('409 recovers current authority and aborts the stale command without replaying it', async t => {
  let current = session(), conflict = false
  const f = fixture(async (url, config) => {
    if (url === '/auth/session') return json(200, current)
    const body = JSON.parse(config.body)
    if (body.membershipId && !conflict) {
      conflict = true; current = session({ version: '2', context: otherIdentity })
      return json(409, {})
    }
    assert.equal(body.membershipId, undefined)
    return json(200, context(current.context, current.version))
  })
  t.after(() => f.coordinator.dispose())
  await f.coordinator.initialize()
  await assert.rejects(f.coordinator.selectContext(otherIdentity.organizationId, otherIdentity.membershipId, f.fence()), error => error.name === 'AbortError')
  assert.equal(f.coordinator.current().session.version, '2')
  assert.equal(f.coordinator.current().record.identity.membershipId, otherIdentity.membershipId)
  assert.equal(f.calls.filter(call => call.config.body && JSON.parse(call.config.body).membershipId).length, 1)
})

for (const poison of ['changed-proof', 'refresh-secret', 'server-revision', 'wrong-selected-member']) test(`poisoned ${poison} bridge never installs a usable context`, async t => {
  let poisoned = false
  const f = fixture(async (url, config) => {
    if (url === '/auth/session') return json(200, session())
    if (!poisoned) return json(200, context())
    const patch = poison === 'refresh-secret' ? { refreshToken: 'owned-unexpected-secret' }
      : poison === 'server-revision' ? { version: '2' } : {}
    return json(200, context(poison === 'changed-proof' ? otherIdentity : identity(), poison === 'wrong-selected-member' ? '2' : '1', patch))
  })
  t.after(() => f.coordinator.dispose())
  await f.coordinator.initialize(); poisoned = true
  const operation = poison === 'wrong-selected-member'
    ? f.coordinator.selectContext(otherIdentity.organizationId, otherIdentity.membershipId, f.fence())
    : f.coordinator.renew(f.fence())
  await assert.rejects(operation, error => error.code === 'SESSION_UNKNOWN')
  assert.equal(f.coordinator.current().record.token, null)
  assert.equal(f.coordinator.current().record.identity.membershipId, identity().membershipId)
})

test('GET rejects malformed CSRF and nonemployee or disabled choices instead of repairing them locally', async t => {
  for (const patch of [
    { csrfToken: 'noncanonical-csrf' },
    { memberships: [member(identity({ memberType: 'CUSTOMER' }))], context: null },
    { memberships: [{ ...member(identity()), status: 'DISABLED' }], context: null },
  ]) {
    const f = fixture(async () => json(200, session(patch)))
    t.after(() => f.coordinator.dispose())
    await assert.rejects(f.coordinator.initialize(), error => error.code === 'SESSION_UNKNOWN')
    assert.equal(f.coordinator.current().record.token, null)
  }
})

test('cross-page invalidation recovers before an ignored-abort transport releases its stale renewal', { timeout: 10_000 }, async t => {
  let current = session(), delayed = false
  const ready = deferred(), release = deferred()
  const f = fixture(async url => {
    if (url === '/auth/session') return json(200, current)
    if (delayed) { delayed = false; ready.resolve(); await release.promise; return json(200, context(identity(), '1', { accessToken: 'owned-stale' })) }
    return json(200, context(current.context, current.version))
  })
  t.after(() => f.coordinator.dispose())
  await f.coordinator.initialize(); delayed = true
  const renewal = f.coordinator.renew(f.fence())
  const rejected = assert.rejects(renewal, error => error.name === 'AbortError')
  await ready.promise
  current = session({ version: '2', context: otherIdentity })
  f.callbacks().invalidated({ sessionId: id(7), revision: '2', invalid: false })
  assert.equal(f.coordinator.current().record.token, null)
  await rejected
  await f.coordinator.reconcile()
  assert.equal(f.coordinator.current().record.identity.membershipId, otherIdentity.membershipId)
  release.resolve(); await settle()
  assert.equal(f.coordinator.current().record.identity.membershipId, otherIdentity.membershipId)
  assert.notEqual(f.coordinator.current().record.token, 'owned-stale')
})

test('pagehide clears secrets and pageshow restores before an ignored-abort response releases', { timeout: 10_000 }, async t => {
  let delayed = false
  const ready = deferred(), release = deferred()
  const f = fixture(async url => {
    if (url === '/auth/session') return json(200, session())
    if (delayed) { delayed = false; ready.resolve(); await release.promise; return json(200, context(identity(), '1', { accessToken: 'owned-hidden-late' })) }
    return json(200, context())
  })
  t.after(() => f.coordinator.dispose())
  await f.coordinator.initialize(); delayed = true
  const renewal = f.coordinator.renew(f.fence())
  const rejected = assert.rejects(renewal, error => error.name === 'AbortError')
  await ready.promise; f.callbacks().suspended()
  assert.equal(f.coordinator.current().record.token, null); assert.equal(f.coordinator.current().tokenExpiresAt, null)
  await rejected
  f.callbacks().focused(); await f.coordinator.reconcile()
  assert.equal(f.coordinator.current().status, 'active')
  release.resolve(); await settle()
  assert.equal(f.coordinator.current().status, 'active'); assert.notEqual(f.coordinator.current().record.token, 'owned-hidden-late')
})

for (const stalled of ['transport', 'body']) test(`request deadline releases a stalled ${stalled} and permits authoritative recovery`, { timeout: 10_000 }, async t => {
  let delayed = false
  const ready = deferred(), release = deferred()
  const f = fixture(async url => {
    if (url === '/auth/session') return json(200, session())
    if (delayed) {
      delayed = false
      if (stalled === 'transport') { ready.resolve(); await release.promise; return json(200, context(identity(), '1', { accessToken: 'owned-timeout-late' })) }
      return { ok: true, status: 200, json: async () => { ready.resolve(); await release.promise; return context(identity(), '1', { accessToken: 'owned-timeout-late' }) } }
    }
    return json(200, context())
  })
  t.after(() => f.coordinator.dispose())
  await f.coordinator.initialize(); delayed = true
  const renewal = f.coordinator.renew(f.fence())
  const rejected = assert.rejects(renewal, error => error.status === 503)
  await ready.promise; f.advance(20_000); await rejected
  assert.equal(f.coordinator.current().record.token, null)
  await f.coordinator.reconcile()
  assert.equal(f.coordinator.current().status, 'active')
  release.resolve(); await settle()
  assert.notEqual(f.coordinator.current().record.token, 'owned-timeout-late')
})

for (const scope of ['PLATFORM', 'GLOBAL']) test(`${scope} logout executes the remote Cookie/CSRF command and does not navigate`, async t => {
  const f = fixture(async (url, config) => {
    if (url === '/auth/session') return json(200, session({ context: null, memberships: [] }))
    assert.equal(url, '/auth/logout'); assert.equal(config.headers['X-CSRF-Token'], csrf)
    assert.deepEqual(JSON.parse(config.body), { expectedVersion: '1', scope })
    return json(200, { redirectTo: '/login?signedOut=1' })
  })
  t.after(() => f.coordinator.dispose())
  await f.coordinator.initialize()
  const before = f.fence()
  assert.equal(await f.coordinator.logout(before, scope), true)
  assert.equal(f.coordinator.current().status, 'anonymous')
  assert.equal(await f.coordinator.forget(before), false)
  assert.equal(f.notices.at(-1).invalid, true)
})

test('logout rejects an untrusted redirect and local forget never sends a remote command', async t => {
  const f = fixture(async url => url === '/auth/session' ? json(200, session({ context: null })) : json(200, { redirectTo: 'https://unregistered.invalid/' }))
  t.after(() => f.coordinator.dispose())
  await f.coordinator.initialize()
  await assert.rejects(f.coordinator.logout(f.fence()), error => error.code === 'SESSION_UNKNOWN')
  const count = f.calls.length
  assert.equal(await f.coordinator.forget(f.fence()), true)
  assert.equal(f.calls.length, count); assert.equal(f.coordinator.current().status, 'anonymous')
})

test('dispose clears timers, observers and pending authority without accepting late responses', async () => {
  const f = fixture(async url => url === '/auth/session' ? json(200, session()) : json(200, context()))
  await f.coordinator.initialize()
  assert(f.timers.size > 0)
  f.coordinator.dispose()
  assert.equal(f.timers.size, 0); assert.equal(f.unsubscribed(), true)
  assert.equal(f.coordinator.current().record, null)
  await assert.rejects(f.coordinator.reconcile(), error => error.name === 'AbortError')
})
