import assert from 'node:assert/strict'
import test from 'node:test'
import fs from 'node:fs'
import vm from 'node:vm'
import path from 'node:path'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
import { webcrypto } from 'node:crypto'

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
    setTimeout, clearTimeout, crypto: webcrypto })
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
    returnedSwitch: options.returnedSwitch,
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
    const body=JSON.parse(config.body),target=new URL(body.returnPath,'https://owned.test')
    assert.equal(body.expectedVersion,'1');assert.equal(target.pathname,'/dashboard')
    assert.match(target.searchParams.get('__platform_context_switch'),new RegExp(`^${id(7)}\\.1\\.[a-f0-9]{32}$`))
    return json(200, { redirectTo: 'https://auth.example.test/oauth/authorize?context_request=owned' })
  }, { navigate: url => { destination = url } })
  t.after(() => f.coordinator.dispose())
  await f.coordinator.initialize()
  await f.coordinator.beginContextSwitch(f.fence(), '/dashboard')
  assert.equal(destination, 'https://auth.example.test/oauth/authorize?context_request=owned')
  assert.equal(f.coordinator.current().status, 'switching')
  assert.equal(f.coordinator.current().record?.token, null)
  assert.equal(f.calls.filter(call => call.url === '/auth/context-switch').length, 1)
})

for (const target of [
  'https://auth.example.test/oidc/authorize?context_request=owned',
  'https://auth.example.test/other-path',
  'http://auth.example.test/oauth/authorize?context_request=owned',
  'https://user:secret@auth.example.test/oauth/authorize?context_request=owned',
  'https://auth.example.test/oauth/authorize?context_request=owned#unexpected',
]) test(`centralized switch rejects an untrusted navigation target: ${new URL(target).pathname} ${new URL(target).protocol}`, async t => {
  let navigated = false
  const f = fixture(async url => {
    if (url === '/auth/session') return json(200, session({ centralizedContextSelection: true }))
    if (url === '/auth/context-token') return json(200, context())
    return json(200, { redirectTo: target })
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
  assert.equal(f.notices.length, 4)
  assert.equal(f.notices.filter(notice => notice.phase === 'switching').length, 2)
  for (const notice of f.notices) assert.deepEqual(Object.keys(notice).sort(), notice.switchId ? ['invalid', 'phase', 'revision', 'sessionId','switchId'] : ['invalid', 'revision', 'sessionId'])
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

test('Phase18 switching notices synchronously freeze another tab and a racing GET cannot restore the old revision', async t => {
  let bridges = 0, current = session({ centralizedContextSelection: true })
  const f = fixture(async url => url === '/auth/session' ? json(200,current) : (bridges++,json(200,context(current.context,current.version))))
  t.after(() => f.coordinator.dispose())
  await f.coordinator.initialize()
  f.callbacks().invalidated({sessionId:id(7),revision:'1',invalid:false,phase:'switching',switchId:'c'.repeat(32)})
  assert.equal(f.coordinator.current().status,'switching')
  assert.equal(f.coordinator.current().record.token,null)
  f.callbacks().focused(); await f.coordinator.reconcile()
  assert.equal(f.coordinator.current().status,'switching')
  assert.equal(bridges,1,'a GET from before server freeze cannot renew the old authority')
  current=session({centralizedContextSelection:true,version:'2',context:otherIdentity})
  await f.coordinator.reconcile()
  assert.equal(f.coordinator.current().record.identity.membershipId,otherIdentity.membershipId)
})

for (const notice of [
  {sessionId:id(99),revision:'1',invalid:false},
  {sessionId:id(99),revision:'1',invalid:true,phase:'settled'},
  {sessionId:id(7),revision:'2',invalid:false},
  {sessionId:id(7),revision:'1',invalid:false},
  {sessionId:id(7),revision:'1',invalid:false,switchId:'b'.repeat(32)},
  undefined,
]) test(`Phase18 unrelated terminal notice cannot release an active switch barrier (${notice?.sessionId ?? 'unbound'}/${notice?.revision ?? 'unbound'}/${notice?.phase ?? 'implicit'}/${notice?.switchId ?? 'uncorrelated'}/${notice?.invalid ?? 'unbound'})`, async t => {
  let bridges=0,current=session({centralizedContextSelection:true})
  const f=fixture(async url=>url==='/auth/session'?json(200,current):
    (bridges++,json(200,context(current.context,current.version,{sessionId:current.sessionId}))))
  t.after(()=>f.coordinator.dispose())
  await f.coordinator.initialize()
  f.callbacks().invalidated({sessionId:id(7),revision:'1',invalid:false,phase:'switching',switchId:'c'.repeat(32)})
  f.callbacks().invalidated(notice)
  await f.coordinator.reconcile()
  assert.equal(f.coordinator.current().status,'switching','an unrelated hint cannot unfreeze the current original revision')
  assert.equal(f.coordinator.current().record.token,null)
  assert.equal(bridges,1,'the old source must not receive another bearer before the switch settles')
  // The notice remains a request to read authority. A genuine replacement
  // session releases the old barrier only after the GET and bridge prove it.
  current=session({centralizedContextSelection:true,sessionId:id(8),context:otherIdentity})
  await f.coordinator.reconcile()
  assert.equal(f.coordinator.current().status,'active')
  assert.equal(f.coordinator.current().record.sessionId,`${id(8)}:1`)
  assert.equal(f.coordinator.current().record.identity.membershipId,otherIdentity.membershipId)
  assert.equal(bridges,2)
})

test('Phase18 a real same-revision initialization notice from another tab cannot settle the current switch',async t=>{
  const handler=async url=>url==='/auth/session'?json(200,session({centralizedContextSelection:true})):json(200,context())
  const current=fixture(handler),newTab=fixture(handler)
  t.after(()=>{current.coordinator.dispose();newTab.coordinator.dispose()})
  await current.coordinator.initialize()
  current.callbacks().invalidated({sessionId:id(7),revision:'1',invalid:false,phase:'switching',switchId:'c'.repeat(32)})
  await newTab.coordinator.initialize()
  const ordinaryNotice=newTab.notices.at(-1)
  assert.equal(ordinaryNotice.sessionId,id(7));assert.equal(ordinaryNotice.revision,'1')
  current.callbacks().invalidated(ordinaryNotice)
  await current.coordinator.reconcile()
  assert.equal(current.coordinator.current().status,'switching','a freshly opened tab emits no completion for this switch')
  assert.equal(current.coordinator.current().record.token,null)
  assert.equal(current.calls.filter(call=>call.url==='/auth/context-token').length,1)
})

test('Phase18 repeated switches at the same revision have distinct correlation and reject the prior completion',async t=>{
  const handler=async url=>url==='/auth/session'?json(200,session({centralizedContextSelection:true})):url==='/auth/context-switch'?json(403,{}):json(200,context())
  const sender=fixture(handler,{navigate:()=>{throw new Error('rejected switch cannot navigate')}}),receiver=fixture(handler)
  t.after(()=>{sender.coordinator.dispose();receiver.coordinator.dispose()})
  await sender.coordinator.initialize();await receiver.coordinator.initialize()
  const completed=[]
  for(let attempt=0;attempt<2;attempt++){
    await assert.rejects(sender.coordinator.beginContextSwitch(sender.fence(),'/dashboard'),error=>error.status===403)
    const started=sender.notices.findLast(notice=>notice.phase==='switching')
    await sender.coordinator.reconcile()
    completed.push({started,terminal:sender.notices.at(-1)})
  }
  assert.notEqual(completed[0].started.switchId,completed[1].started.switchId)
  assert.equal(completed[0].started.sessionId,completed[1].started.sessionId)
  assert.equal(completed[0].started.revision,completed[1].started.revision)
  receiver.callbacks().invalidated(completed[1].started)
  receiver.callbacks().invalidated(completed[0].terminal)
  await receiver.coordinator.reconcile()
  assert.equal(receiver.coordinator.current().status,'switching');assert.equal(receiver.coordinator.current().record.token,null)
  receiver.callbacks().invalidated(completed[1].terminal)
  await receiver.coordinator.reconcile()
  assert.equal(receiver.coordinator.current().status,'active')
})

test('Phase18 concurrent switch completion cannot erase another pending switch at the same revision',async t=>{
  const f=fixture(async url=>url==='/auth/session'?json(200,session()):json(200,context()))
  t.after(()=>f.coordinator.dispose());await f.coordinator.initialize()
  const first={sessionId:id(7),revision:'1',invalid:false,phase:'switching',switchId:'a'.repeat(32)}
  const second={...first,switchId:'b'.repeat(32)}
  f.callbacks().invalidated(first);f.callbacks().invalidated(second)
  f.callbacks().invalidated({...second,phase:'settled'})
  await f.coordinator.reconcile()
  assert.equal(f.coordinator.current().status,'switching','settling B cannot discard pending A')
  assert.equal(f.coordinator.current().record.token,null)
  f.callbacks().invalidated({...first,phase:'settled'})
  await f.coordinator.reconcile()
  assert.equal(f.coordinator.current().status,'active')
})

test('Phase18 a newer pending revision survives an intermediate authoritative revision',async t=>{
  let current=session()
  const f=fixture(async url=>url==='/auth/session'?json(200,current):json(200,context(current.context,current.version)))
  t.after(()=>f.coordinator.dispose());await f.coordinator.initialize()
  const first={sessionId:id(7),revision:'1',invalid:false,phase:'switching',switchId:'a'.repeat(32)}
  const second={...first,revision:'2',switchId:'b'.repeat(32)}
  f.callbacks().invalidated(first);f.callbacks().invalidated(second)
  current=session({version:'2',context:otherIdentity});await f.coordinator.reconcile()
  assert.equal(f.coordinator.current().status,'switching');assert.equal(f.coordinator.current().record.token,null)
  current=session({version:'3',context:otherIdentity});await f.coordinator.reconcile()
  assert.equal(f.coordinator.current().status,'active');assert.equal(f.coordinator.current().session.version,'3')
})

test('Phase18 settled memory ignores a delayed start without erasing another pending switch',async t=>{
  const f=fixture(async url=>url==='/auth/session'?json(200,session()):json(200,context()))
  t.after(()=>f.coordinator.dispose());await f.coordinator.initialize()
  const old={sessionId:id(7),revision:'1',invalid:false,phase:'settled',switchId:'a'.repeat(32)}
  f.callbacks().invalidated(old);await f.coordinator.reconcile()
  const current={...old,phase:'switching',switchId:'b'.repeat(32)}
  f.callbacks().invalidated(current);f.callbacks().invalidated({...old,phase:'switching'})
  f.callbacks().invalidated(old);await f.coordinator.reconcile()
  assert.equal(f.coordinator.current().status,'switching')
  f.callbacks().invalidated({...current,phase:'settled'});await f.coordinator.reconcile()
  assert.equal(f.coordinator.current().status,'active')
})

test('Phase18 pending capacity overflow never evicts an unresolved barrier or ignores its newer revision',async t=>{
  let current=session(),bridges=0
  const f=fixture(async url=>url==='/auth/session'?json(200,current):(bridges++,json(200,context(current.context,current.version))))
  t.after(()=>f.coordinator.dispose());await f.coordinator.initialize()
  const operations=Array.from({length:34},(_,index)=>({sessionId:id(7),revision:index<33?'1':'2',invalid:false,phase:'switching',switchId:(index+1).toString(16).padStart(32,'0')}))
  for(const operation of operations)f.callbacks().invalidated(operation)
  for(const operation of operations)f.callbacks().invalidated({...operation,phase:'settled'})
  await f.coordinator.reconcile()
  assert.equal(f.coordinator.current().status,'unknown');assert.equal(f.coordinator.current().record.token,null);assert.equal(bridges,1)
  current=session({version:'2',context:otherIdentity});await f.coordinator.reconcile()
  assert.equal(f.coordinator.current().record.token,null,'r2 does not exceed the untracked overflow barrier at r2')
  current=session({version:'3',context:otherIdentity});await f.coordinator.reconcile()
  assert.equal(f.coordinator.current().status,'active');assert.equal(bridges,2)
})

test('Phase18 settled replay memory is bounded and eviction can only conservatively refreeze',async t=>{
  const f=fixture(async url=>url==='/auth/session'?json(200,session()):json(200,context()))
  t.after(()=>f.coordinator.dispose());await f.coordinator.initialize()
  const operations=Array.from({length:129},(_,index)=>({sessionId:id(7),revision:'1',invalid:false,phase:'settled',switchId:(index+1).toString(16).padStart(32,'0')}))
  for(const operation of operations)f.callbacks().invalidated(operation)
  await f.coordinator.reconcile()
  f.callbacks().invalidated({...operations.at(-1),phase:'switching'})
  assert.equal(f.coordinator.current().status,'active','the newest completed operation is remembered')
  f.callbacks().invalidated({...operations[0],phase:'switching'})
  assert.equal(f.coordinator.current().status,'switching','an evicted old start may pause, never authorize')
  assert.equal(f.coordinator.current().record.token,null)
})

test('Phase18 queued renewal cannot bridge the old source after recovery reports a pending barrier',async t=>{
  const entered=deferred(),release=deferred();let current=session(),bridges=0
  const f=fixture(async(url,config)=>{
    if(url==='/auth/session')return json(200,current)
    if(JSON.parse(config.body).membershipId){entered.resolve();await release.promise;return json(403,{})}
    bridges++;return json(200,context(current.context,current.version))
  })
  t.after(()=>f.coordinator.dispose());await f.coordinator.initialize();const original=f.fence()
  const selected=f.coordinator.selectContext(otherIdentity.organizationId,otherIdentity.membershipId,original)
  const selectionRejected=assert.rejects(selected,error=>error.status===403)
  await entered.promise
  const renewal=f.coordinator.renew(original)
  const renewalRejected=assert.rejects(renewal,error=>error.name==='AbortError')
  f.callbacks().invalidated({sessionId:id(7),revision:'1',invalid:false,phase:'switching',switchId:'a'.repeat(32)})
  release.resolve();await selectionRejected;await renewalRejected
  await f.coordinator.reconcile();await settle()
  assert.equal(bridges,1,'queued renew cannot issue an old bridge after restore stayed frozen')
  assert.equal(f.coordinator.current().record.token,null)
  current=session({version:'2',context:otherIdentity});await f.coordinator.reconcile()
  assert.equal(f.coordinator.current().session.version,'2')
  assert.equal(await f.coordinator.renew(f.fence()),'owned-access-2')
})

test('Phase18 UNKNOWN barrier blocks every coordinator mutation until authoritative recovery',async t=>{
  let current=session({centralizedContextSelection:true}),bridges=0
  const f=fixture(async url=>url==='/auth/session'?json(200,current):url==='/auth/context-switch'?json(503,{}):(bridges++,json(200,context(current.context,current.version))))
  t.after(()=>f.coordinator.dispose());await f.coordinator.initialize()
  await assert.rejects(f.coordinator.beginContextSwitch(f.fence(),'/dashboard'),error=>error.status===503)
  const expected=f.fence(),operation=f.notices.find(notice=>notice.phase==='switching')
  for(const action of [()=>f.coordinator.renew(expected),()=>f.coordinator.selectContext(otherIdentity.organizationId,otherIdentity.membershipId,expected),
    ()=>f.coordinator.beginContextSwitch(expected,'/dashboard'),()=>f.coordinator.logout(expected)]) await assert.rejects(action(),error=>error.name==='AbortError')
  assert.equal(bridges,1);assert.equal(f.calls.filter(call=>call.url==='/auth/context-switch').length,1)
  assert.equal(f.coordinator.current().record.token,null)
  assert.equal(f.notices.filter(notice=>notice.phase==='settled'&&notice.switchId===operation.switchId).length,0)
  current=session({centralizedContextSelection:true,version:'2',context:otherIdentity});await f.coordinator.reconcile()
  assert.equal(await f.coordinator.renew(f.fence()),'owned-access-2')
})

test('Phase18 local credential removal cannot announce completion of an unknown remote switch',async t=>{
  const handler=async url=>url==='/auth/session'?json(200,session({centralizedContextSelection:true})):url==='/auth/context-switch'?json(503,{}):json(200,context())
  const sender=fixture(handler),receiver=fixture(handler)
  t.after(()=>{sender.coordinator.dispose();receiver.coordinator.dispose()})
  await sender.coordinator.initialize();await receiver.coordinator.initialize()
  await assert.rejects(sender.coordinator.beginContextSwitch(sender.fence(),'/dashboard'),error=>error.status===503)
  const operation=sender.notices.find(notice=>notice.phase==='switching')
  receiver.callbacks().invalidated(operation)
  assert.equal(await sender.coordinator.forget(sender.fence()),true)
  assert.equal(sender.coordinator.current().status,'anonymous')
  assert.equal(sender.notices.filter(notice=>notice.phase==='settled'&&notice.switchId===operation.switchId).length,0)
  await receiver.coordinator.reconcile();assert.equal(receiver.coordinator.current().status,'switching')
})

test('Phase18 simultaneous active tabs record each other and settle only their own rejected POST',async t=>{
  const firstEntered=deferred(),secondEntered=deferred(),firstRelease=deferred(),secondRelease=deferred();let firstSignal,secondSignal,secondBridges=0
  const first=fixture(async(url,config)=>{
    if(url==='/auth/session')return json(200,session({centralizedContextSelection:true}))
    if(url==='/auth/context-token')return json(200,context())
    firstSignal=config.signal;firstEntered.resolve();await firstRelease.promise;return json(403,{})
  }),second=fixture(async(url,config)=>{
    if(url==='/auth/session')return json(200,session({centralizedContextSelection:true}))
    if(url==='/auth/context-token'){secondBridges++;return json(200,context())}
    secondSignal=config.signal;secondEntered.resolve();await secondRelease.promise;return json(403,{})
  })
  t.after(()=>{first.coordinator.dispose();second.coordinator.dispose()})
  await first.coordinator.initialize();await second.coordinator.initialize()
  const firstPost=first.coordinator.beginContextSwitch(first.fence(),'/dashboard'),firstRejected=assert.rejects(firstPost,error=>error.status===403)
  const secondPost=second.coordinator.beginContextSwitch(second.fence(),'/dashboard'),secondRejected=assert.rejects(secondPost,error=>error.status===403)
  await firstEntered.promise;await secondEntered.promise
  const firstOperation=first.notices.find(notice=>notice.phase==='switching'),secondOperation=second.notices.find(notice=>notice.phase==='switching')
  first.callbacks().invalidated(secondOperation);second.callbacks().invalidated(firstOperation)
  assert.equal(firstSignal.aborted,false);assert.equal(secondSignal.aborted,false)
  secondRelease.resolve();await secondRejected;await second.coordinator.reconcile()
  assert.equal(second.coordinator.current().status,'switching','rejected B must still remember pending A')
  assert.equal(secondBridges,1,'authenticated source GET can settle only B; it cannot bridge while A waits')
  const secondTerminal=second.notices.at(-1);assert.equal(secondTerminal.switchId,secondOperation.switchId)
  first.callbacks().invalidated(secondTerminal)
  firstRelease.resolve();await firstRejected;await first.coordinator.reconcile()
  second.callbacks().invalidated(first.notices.at(-1));await second.coordinator.reconcile()
  assert.equal(first.coordinator.current().status,'active');assert.equal(second.coordinator.current().status,'active')
})

test('Phase18 foreign hints cannot abort an active local switch POST or settle its original revision',async t=>{
  const entered=deferred(),release=deferred();let postSignal,bridges=0,current=session({centralizedContextSelection:true}),destination
  const f=fixture(async(url,config)=>{
    if(url==='/auth/session')return json(200,current)
    if(url==='/auth/context-token'){bridges++;return json(200,context(current.context,current.version))}
    postSignal=config.signal;entered.resolve();await release.promise
    return json(200,{redirectTo:'https://auth.example.test/oauth/authorize?owned=active-post'})
  },{navigate:url=>{destination=url}})
  t.after(()=>f.coordinator.dispose());await f.coordinator.initialize()
  const pending=f.coordinator.beginContextSwitch(f.fence(),'/dashboard');await entered.promise
  const operation=f.notices.find(notice=>notice.phase==='switching')
  assert.match(operation.switchId,/^[a-f0-9]{32}$/)
  for(const notice of [{sessionId:id(99),revision:'1',invalid:false},{...operation,phase:'settled'},{...operation,switchId:'b'.repeat(32),phase:'switching'}]) f.callbacks().invalidated(notice)
  assert.equal(postSignal.aborted,false,'even a copied correlation hint cannot interrupt the active local request')
  assert.equal(f.coordinator.current().record.token,null);assert.equal(bridges,1)
  release.resolve();await pending;await f.coordinator.reconcile()
  assert.equal(destination,'https://auth.example.test/oauth/authorize?owned=active-post')
  assert.equal(f.coordinator.current().status,'switching');assert.equal(bridges,1)
  current=session({centralizedContextSelection:true,version:'2',context:otherIdentity})
  await f.coordinator.reconcile()
  assert.equal(f.coordinator.current().status,'active')
  assert.equal(f.notices.at(-1).sessionId,operation.sessionId)
  assert.equal(f.notices.at(-1).revision,operation.revision)
  assert.equal(f.notices.at(-1).switchId,operation.switchId)
})

for(const mode of ['503','timeout'])test(`Phase18 unknown switch outcome retains the original barrier after ${mode}`,async t=>{
  const entered=deferred(),stalled=deferred();let bridges=0
  const f=fixture(async url=>{
    if(url==='/auth/session')return json(200,session({centralizedContextSelection:true}))
    if(url==='/auth/context-token'){bridges++;return json(200,context())}
    entered.resolve();return mode==='503'?json(503,{}):stalled.promise
  },{navigate:()=>{throw new Error('unknown outcomes must never navigate')}})
  t.after(()=>f.coordinator.dispose());await f.coordinator.initialize()
  const pending=f.coordinator.beginContextSwitch(f.fence(),'/dashboard')
  const rejected=assert.rejects(pending,error=>error.status===503)
  await entered.promise;if(mode==='timeout')f.advance(20_000);await rejected
  const operation=f.notices.find(notice=>notice.phase==='switching')
  f.callbacks().invalidated({...operation,phase:'settled'})
  await f.coordinator.reconcile()
  assert.equal(f.coordinator.current().status,'unknown')
  assert.equal(f.coordinator.current().record.token,null);assert.equal(bridges,1)
  assert.equal(f.notices.filter(notice=>notice.switchId===operation.switchId&&notice.phase==='settled').length,0)
})

for(const live of [true,false])test(`Phase18 a definite rejected local switch settles only after original source proof (${live})`,async t=>{
  let allowed=true
  const f=fixture(async url=>url==='/auth/context-switch'?json(403,{}):!allowed?json(401,{}):url==='/auth/session'?json(200,session({centralizedContextSelection:true})):json(200,context()),{navigate:()=>{throw new Error('rejection cannot navigate')}})
  t.after(()=>f.coordinator.dispose());await f.coordinator.initialize()
  await assert.rejects(f.coordinator.beginContextSwitch(f.fence(),'/dashboard'),error=>error.status===403)
  const operation=f.notices.find(notice=>notice.phase==='switching');allowed=live
  await f.coordinator.reconcile()
  assert.equal(f.coordinator.current().status,live?'active':'anonymous')
  assert.equal(f.notices.at(-1).switchId,operation.switchId)
  assert.equal(f.notices.at(-1).invalid,!live)
})

for(const state of ['original','new-session','anonymous'])test(`Phase18 returned correlation preserves original switch tuple and rechecks source (${state})`,async t=>{
  const operation={sessionId:id(7),revision:'1',switchId:'c'.repeat(32)}
  const current=state==='new-session'?session({sessionId:id(8),version:'2',context:otherIdentity}):session()
  const f=fixture(async url=>state==='anonymous'?json(401,{}):url==='/auth/session'?json(200,current):json(200,context(current.context,current.version,{sessionId:current.sessionId})),{returnedSwitch:operation})
  t.after(()=>f.coordinator.dispose());await f.coordinator.initialize()
  const terminal=f.notices.at(-1)
  for(const key of ['sessionId','revision','switchId'])assert.equal(terminal[key],operation[key])
  assert.equal(terminal.phase,'settled');assert.equal(terminal.invalid,state==='anonymous')
  assert.equal(f.coordinator.current().status,state==='anonymous'?'anonymous':'active')
  assert.equal(f.calls.filter(call=>call.url==='/auth/context-token').length,state==='anonymous'?0:1)
})

test('Phase18 switch return marker never enables an external, duplicate or oversized return path',async t=>{
  for(const path of ['https://other.example/','//other.example/','/dashboard?__platform_context_switch=forged','/dashboard#fragment','/bad\\path','/bad\u0001path','/'+ 'a'.repeat(2048)]){
    const f=fixture(async url=>url==='/auth/session'?json(200,session({centralizedContextSelection:true})):json(200,context()),{navigate:()=>{throw new Error('invalid return cannot navigate')}})
    t.after(()=>f.coordinator.dispose());await f.coordinator.initialize()
    await assert.rejects(f.coordinator.beginContextSwitch(f.fence(),path),error=>error.code==='INVALID_CONTEXT_RETURN')
    assert.equal(f.calls.filter(call=>call.url==='/auth/context-switch').length,0)
  }
})

for (const live of [true,false]) test(`Phase18 cancellation only resumes a live original source (${live})`, async t => {
  let allowed=true
  const f=fixture(async url => !allowed ? json(401,{}) : url==='/auth/session' ? json(200,session()) : json(200,context()))
  t.after(() => f.coordinator.dispose())
  await f.coordinator.initialize()
  f.callbacks().invalidated({sessionId:id(7),revision:'1',invalid:false,phase:'switching',switchId:'c'.repeat(32)})
  allowed=live
  f.callbacks().invalidated({sessionId:id(7),revision:'1',invalid:false,switchId:'c'.repeat(32)})
  await f.coordinator.reconcile()
  assert.equal(f.coordinator.current().status,live?'active':'anonymous')
  assert.equal(f.coordinator.current().record.token,live?'owned-access-1':null)
})

test('Phase18 delayed notices from another account or an older revision cannot freeze the replacement', async t => {
  const f=fixture(async url=>url==='/auth/session'?json(200,session({version:'3'})):json(200,context(identity(),'3')))
  t.after(()=>f.coordinator.dispose())
  await f.coordinator.initialize()
  for(const notice of [{sessionId:id(99),revision:'9',invalid:true,phase:'switching'},{sessionId:id(7),revision:'2',invalid:false,phase:'switching'}]) f.callbacks().invalidated(notice)
  assert.equal(f.coordinator.current().status,'active')
  assert.equal(f.coordinator.current().record.token,'owned-access-3')
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
