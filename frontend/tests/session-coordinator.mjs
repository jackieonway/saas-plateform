import assert from 'node:assert/strict'
import test from 'node:test'
import { fixture, sharedOrigin, deferred, json, identity, targetIdentity, contextResponse, SESSION_KEY } from './helpers/session-fixture.mjs'
import { contextIdentityKey } from '../src/lib/context-identity.ts'

const key = SESSION_KEY
const read = origin => JSON.parse(origin.storage.get(key))
const tick = async () => { await Promise.resolve(); await Promise.resolve(); await Promise.resolve() }

test('two independent pages consume the same expired refresh once and adopt one complete pair', async () => {
  const origin = sharedOrigin({ events: false }), start = deferred(), release = deferred()
  let refreshes = 0
  const handler = async (url, options) => {
    if (url === '/oauth/token') { refreshes++; start.resolve(); await release.promise; return json(200, { access_token: 'new', refresh_token: 'psr_new' }) }
    return options.headers.Authorization === 'Bearer new' ? json(200, { ok: true }) : json(401, {})
  }
  const a = await origin.page(handler); await a.state().setSession('expired', 'psr_old', undefined, identity())
  const b = await origin.page(handler), id = a.state().sessionId
  const first = a.api.api.get('/api/a'), second = b.api.api.get('/api/b')
  await start.promise
  assert.equal(read(origin).status, 'in-flight'); assert.equal(read(origin).intent.kind, 'refresh')
  release.resolve(); assert.equal((await first).ok, true); assert.equal((await second).ok, true)
  assert.equal(refreshes, 1); assert.equal(a.state().sessionId, id); assert.equal(b.state().sessionId, id)
  assert.equal(a.state().refreshToken, 'psr_new'); assert.equal(b.state().refreshToken, 'psr_new')
})

test('unproven v1 and legacy pairs become anonymous under the lock and cannot resurrect', async () => {
  const origin = sharedOrigin({ initial: [['platform-session-v1', JSON.stringify({ schema: 1, version: 7, sessionId: 'old-session', status: 'active', token: 'v1-token', refreshToken: 'psr_v1' })], ['platform-token', 'legacy'], ['platform-refresh-token', 'psr_legacy']] })
  const a = await origin.page(async () => assert.fail('ambiguous credentials must never reach refresh')), b = await origin.page(async () => assert.fail('legacy credentials cannot authorize any request'))
  assert.equal(a.state().token, null); assert.equal(a.state().sessionId, null); assert.equal(b.state().sessionStatus, 'anonymous')
  assert.equal(read(origin).identity, null); assert.equal(read(origin).contextKey, null); assert.equal(read(origin).schema, 2)
  assert.equal(origin.storage.has('platform-session-v1'), false)
  assert.equal(origin.storage.has('platform-token'), false); assert.equal(origin.storage.has('platform-refresh-token'), false)
  await a.state().logout()
  origin.storage.set('platform-token', 'stale'); origin.storage.set('platform-refresh-token', 'psr_stale')
  await b.state().reconcileSession()
  assert.equal(b.state().token, null); assert.equal(read(origin).status, 'anonymous')
  assert.ok(origin.writes.every(write => write.key === key))
})

for (const kind of ['refresh', 'context', 'logout']) test(`a new lock owner quarantines abandoned ${kind} intent without reusing credentials`, async () => {
  const origin = sharedOrigin()
  const a = await origin.page(async () => json(200, {})); await a.state().setSession('before', 'psr_before', undefined, identity())
  const base = read(origin)
  origin.storage.set(key, JSON.stringify({ ...base, version: base.version + 1, status: 'in-flight', intent: { opId: 'departed-owner', baseVersion: base.version, kind } }))
  const b = await origin.page(async () => { assert.fail('abandoned credentials must not be sent') })
  assert.equal(b.state().sessionStatus, 'unknown'); assert.equal(b.state().token, null)
  assert.equal(read(origin).token, 'before'); assert.equal(read(origin).refreshToken, 'psr_before')
  await assert.rejects(b.api.api.get('/api/users'), error => error.code === 'SESSION_UNKNOWN')
  await b.state().setSession('new-login', 'psr_login', undefined, identity())
  assert.equal(read(origin).token, 'new-login')
})

test('delayed cross-page events cannot expose an old successful response', async () => {
  const origin = sharedOrigin({ events: false }), start = deferred(), release = deferred()
  const a = await origin.page(async () => { start.resolve(); await release.promise; return json(200, { organization: 'A' }) })
  await a.state().setSession('org-A', 'psr_A', undefined, identity())
  const b = await origin.page(async () => json(200, {}))
  const pending = a.api.api.get('/api/old'); await start.promise
  await b.state().setSession('org-B', 'psr_B', undefined, identity()); release.resolve()
  await assert.rejects(pending, error => error.name === 'AbortError')
  assert.equal(a.state().token, 'org-B')
})

test('shared record is reconciled before reading a response body', async () => {
  const origin = sharedOrigin({ events: false }); let bodyReads = 0, b
  const a = await origin.page(async () => {
    await b.state().setSession('B', 'psr_B', undefined, identity())
    return { status: 200, ok: true, text: async () => { bodyReads++; return '{"organization":"A"}' } }
  })
  await a.state().setSession('A', 'psr_A', undefined, identity()); b = await origin.page(async () => json(200, {}))
  await assert.rejects(a.api.api.get('/api/old'), error => error.name === 'AbortError')
  assert.equal(bodyReads, 0)
})

test('a body that finishes after shared logout is discarded even without event delivery', async () => {
  const origin = sharedOrigin({ events: false }), start = deferred(), body = deferred()
  const a = await origin.page(async () => ({ status: 200, ok: true, text: () => { start.resolve(); return body.promise } }))
  await a.state().setSession('A', 'psr_A', undefined, identity()); const b = await origin.page(async () => json(200, {}))
  const pending = a.api.api.get('/api/body'); await start.promise
  await b.state().logout(); body.resolve('{"organization":"A"}')
  await assert.rejects(pending, error => error.name === 'AbortError')
})

test('captured platform bearer fence rejects A after B replaces shared credentials before any send', async () => {
  const f = await fixture(async () => { assert.fail('stale platform request sent') })
  const fence = f.capturePlatformSessionFence('expired-access')
  await f.state().setSession('B', 'psr_B', undefined, identity())
  assert.throws(() => f.capturePlatformSessionFence('expired-access'), error => error.name === 'AbortError')
  await assert.rejects(f.api.api.get('/api/users', { bearer: 'expired-access', sessionFence: fence }), error => error.name === 'AbortError')
})

test('explicit credential or application bearer never refreshes even if its string equals the current token', async () => {
  const f = await fixture(async () => json(401, {}))
  await assert.rejects(f.api.api.get('/api/apps/example/context', { bearer: 'expired-access' }), error => error.status === 401)
  assert.equal(f.calls.length, 1); assert.equal(f.state().token, 'expired-access')
})

for (const malformed of ['missing-pair', 'unreadable-body']) test(`a refresh success with ${malformed} is unknown and cannot replay`, async () => {
  let count = 0
  const f = await fixture(async url => {
    if (url !== '/oauth/token') return json(401, {})
    count++; return malformed === 'missing-pair' ? json(200, { access_token: 'new' }) : new Response('unreadable', { status: 200 })
  })
  await assert.rejects(f.api.api.get('/api/a'), error => error.code === 'SESSION_UNKNOWN')
  await assert.rejects(f.api.api.get('/api/b'), error => error.code === 'SESSION_UNKNOWN')
  assert.equal(count, 1); assert.equal(f.record().refreshToken, 'psr_test-refresh'); assert.equal(f.state().profile, null)
})

test('an actual operation deadline releases the lock, preserves unknown credentials and fences late refresh', async () => {
  const start = deferred(), release = deferred()
  const f = await fixture(async url => {
    if (url === '/oauth/token') { start.resolve(); await release.promise; return json(200, { access_token: 'late', refresh_token: 'psr_late' }) }
    return json(401, {})
  }, { controlDeadline: true })
  const rejected = assert.rejects(f.api.api.get('/api/a'), error => error.code === 'SESSION_UNKNOWN')
  await start.promise; f.fireDeadline(); await rejected
  assert.equal(f.record().status, 'unknown'); assert.equal(f.record().refreshToken, 'psr_test-refresh')
  await f.state().setSession('new-login', 'psr_login', undefined, identity()); release.resolve(); await tick()
  assert.equal(f.record().token, 'new-login'); assert.equal(f.record().refreshToken, 'psr_login')
})

for (const cancellation of ['deadline', 'replacement']) test(`late context 401 after ${cancellation} cannot start a refresh or retry outside the lock`, async () => {
  const start = deferred(), release = deferred(); let refreshes = 0, contexts = 0
  const f = await fixture(async url => {
    if (url === '/oauth/token') { refreshes++; return json(200, { access_token: 'unexpected', refresh_token: 'unexpected' }) }
    contexts++; start.resolve(); await release.promise; return json(401, {})
  }, { controlDeadline: true })
  const rejected = assert.rejects(f.api.switchPlatformContext(targetIdentity.organizationId, targetIdentity.membershipId), error => error.name === 'AbortError' || error.code === 'SESSION_UNKNOWN')
  await start.promise
  if (cancellation === 'deadline') { f.fireDeadline(); await rejected; await f.state().setSession('new-login', 'psr_login', undefined, identity()) }
  else await f.state().setSession('new-login', 'psr_login', undefined, identity())
  release.resolve(); await rejected; await tick()
  assert.equal(refreshes, 0); assert.equal(contexts, 1); assert.equal(f.record().token, 'new-login')
})

test('stale logout and login callbacks cannot interrupt a replacement session refresh', async () => {
  const start = deferred(), release = deferred()
  const f = await fixture(async (url, options) => {
    if (url === '/oauth/token') { start.resolve(); await release.promise; return json(200, { access_token: 'B-renewed', refresh_token: 'psr_B-new' }) }
    return options.headers.Authorization === 'Bearer B-renewed' ? json(200, { ok: true }) : json(401, {})
  })
  const stale = f.captureSessionFence(); await f.state().setSession('B', 'psr_B', undefined, identity())
  const pending = f.api.api.get('/api/b'); await start.promise
  const oldLogout = f.state().logout(stale), oldLogin = f.state().setSession('late-A', 'psr_A', stale, identity())
  release.resolve(); assert.equal((await pending).ok, true)
  assert.equal(await oldLogout, false); assert.equal(await oldLogin, false)
  assert.equal(f.record().token, 'B-renewed'); assert.equal(f.record().status, 'active')
})

test('captured logical logout remains valid after rotation but never after another login', async () => {
  const f = await fixture(async (url, options) => url === '/oauth/token' ? json(200, { access_token: 'rotated', refresh_token: 'psr_rotated' }) :
    options.headers.Authorization === 'Bearer rotated' ? json(200, {}) : json(401, {}))
  const fence = f.captureSessionFence(); await f.api.api.get('/api/a')
  assert.equal(await f.state().logout(fence), true)
  await f.state().setSession('B', 'psr_B', undefined, identity())
  assert.equal(await f.state().logout(fence), false); assert.equal(f.record().token, 'B')
})

test('login replacement accepts the same logical session after refresh and rejects anonymous ABA', async () => {
  const f = await fixture(async (url, options) => url === '/oauth/token' ? json(200, { access_token: 'rotated', refresh_token: 'psr_rotated' }) :
    options.headers.Authorization === 'Bearer rotated' ? json(200, {}) : json(401, {}))
  const fence = f.captureSessionFence(); await f.api.api.get('/api/a')
  assert.equal(await f.state().setSession('B', 'psr_B', fence, identity()), true)
  await f.state().logout(); const anonymous = f.captureSessionFence()
  await f.state().setSession('C', 'psr_C', undefined, identity()); await f.state().logout()
  assert.equal(await f.state().setSession('late-login', 'late', anonymous, identity()), false)
  assert.equal(f.record().status, 'anonymous')
})

test('context operation immediately isolates both pages and commits one new shared identity', async () => {
  const origin = sharedOrigin({ events: false }), start = deferred(), release = deferred()
  const a = await origin.page(async () => { start.resolve(); await release.promise; return json(200, contextResponse('org-B', 'psr_B')) })
  await a.state().setSession('org-A', 'psr_A', undefined, identity()); const b = await origin.page(async () => json(200, {}))
  const profile = contextResponse('org-A', 'psr_A', identity())
  a.state().setProfile(profile); b.state().setProfile(profile)
  const id = a.state().sessionId, oldEpoch = b.state().contextEpoch
  const pending = a.api.switchPlatformContext(targetIdentity.organizationId, targetIdentity.membershipId); await start.promise; b.reconcileSessionSync()
  assert.equal(a.state().contextSwitching, true); assert.equal(b.state().contextSwitching, true)
  assert.equal(a.state().profile, null); assert.equal(b.state().profile, null); assert.equal(b.state().token, null)
  release.resolve(); await pending; await b.state().reconcileSession()
  assert.equal(a.state().sessionId, b.state().sessionId); assert.notEqual(a.state().sessionId, id)
  assert.equal(b.state().token, 'org-B'); assert.ok(b.state().contextEpoch > oldEpoch)
})

test('a definite rejected context keeps an internally refreshed pair without restoring old credentials', async () => {
  let contexts = 0
  const f = await fixture(async url => {
    if (url === '/oauth/token') return json(200, { access_token: 'renewed', refresh_token: 'psr_renewed' })
    return json(++contexts === 1 ? 401 : 403, { message: 'not a member' })
  })
  const id = f.state().sessionId
  await assert.rejects(f.api.switchPlatformContext(targetIdentity.organizationId, targetIdentity.membershipId), error => error.status === 403)
  assert.equal(contexts, 2); assert.equal(f.record().token, 'renewed'); assert.equal(f.record().refreshToken, 'psr_renewed')
  assert.equal(f.state().sessionId, id); assert.equal(f.state().contextSwitching, false)
})

for (const missing of ['web-locks', 'storage', 'lock-request']) test(`missing ${missing} fails closed with a concrete capability message`, async () => {
  const origin = sharedOrigin({ lockRejected: missing === 'lock-request' })
  const p = await origin.page(async () => { assert.fail('no request is permitted without coordination') },
    { initialize: false, noLocks: missing === 'web-locks', storageUnavailable: missing === 'storage' })
  await assert.rejects(p.state().initializeSessionCoordination(), error => error.code === 'SESSION_UNAVAILABLE')
  assert.equal(p.state().sessionStatus, 'unavailable'); assert.match(p.state().coordinationError, /Web Locks/)
  assert.equal(p.state().token, null); assert.equal(p.calls.length, 0)
})

test('storage events coordinate pages when BroadcastChannel is absent', async () => {
  const origin = sharedOrigin()
  const a = await origin.page(async () => json(200, {})), b = await origin.page(async () => json(200, {}))
  await a.state().setSession('shared', 'psr_shared', undefined, identity()); await tick()
  assert.equal(b.state().token, 'shared'); assert.equal(b.state().sessionId, a.state().sessionId)
  await a.state().logout(); await tick()
  assert.equal(b.state().sessionStatus, 'anonymous'); assert.equal(b.state().token, null)
})

test('lost durable write after a refresh response cannot fall back to usable in-memory credentials', async () => {
  const origin = sharedOrigin({ events: false }), start = deferred(), release = deferred()
  const a = await origin.page(async url => {
    if (url === '/oauth/token') { start.resolve(); await release.promise; return json(200, { access_token: 'consumed', refresh_token: 'psr_consumed' }) }
    return json(401, {})
  })
  await a.state().setSession('expired', 'psr_old', undefined, identity())
  const rejected = assert.rejects(a.api.api.get('/api/a'), error => error.code === 'SESSION_UNAVAILABLE')
  await start.promise; a.denyWrites(); release.resolve(); await rejected
  assert.equal(a.state().token, null); assert.equal(a.state().sessionStatus, 'unavailable')
  const b = await origin.page(async () => { assert.fail('consumed refresh was replayed') })
  assert.equal(b.state().sessionStatus, 'unknown'); assert.equal(read(origin).refreshToken, 'psr_old')
})

test('shared context isolation aborts platform requests without a caller-provided signal', async () => {
  const origin = sharedOrigin({ events: false }), started = deferred(), release = deferred()
  let transportSignal
  const a = await origin.page(async (_url, options) => { transportSignal = options.signal; started.resolve(); await release.promise; return json(200, { old: true }) })
  await a.state().setSession('A', 'psr_A', undefined, identity()); const b = await origin.page(async () => json(200, {}))
  const pending = a.api.api.get('/api/unscoped-mutation'); await started.promise
  assert.equal(transportSignal.aborted, false)
  await b.state().setSession('B', 'psr_B', undefined, identity()); a.reconcileSessionSync()
  assert.equal(transportSignal.aborted, true)
  release.resolve(); await assert.rejects(pending, error => error.name === 'AbortError')
})

test('a logout queued behind another page operation captures its original identity at invocation', async () => {
  const origin = sharedOrigin({ events: false }), started = deferred(), release = deferred()
  const a = await origin.page(async () => { started.resolve(); await release.promise; return json(200, contextResponse('B', 'psr_B')) })
  await a.state().setSession('A', 'psr_A', undefined, identity()); const b = await origin.page(async () => json(200, {}))
  const switching = a.api.switchPlatformContext(targetIdentity.organizationId, targetIdentity.membershipId); await started.promise
  const logout = b.state().logout()
  release.resolve(); await switching
  assert.equal(await logout, false); assert.equal(read(origin).token, 'B')
})

test('an old-page mutation or context intent cannot borrow the identity of an in-flight shared context', async () => {
  const origin = sharedOrigin({ events: false }), started = deferred(), release = deferred()
  let oldPageSends = 0
  const a = await origin.page(async () => { started.resolve(); await release.promise; return json(200, contextResponse('B', 'psr_B')) })
  await a.state().setSession('A', 'psr_A', undefined, identity()); const b = await origin.page(async () => { oldPageSends++; return json(200, {}) })
  const switchB = a.api.switchPlatformContext(targetIdentity.organizationId, targetIdentity.membershipId); await started.promise
  await assert.rejects(b.api.api.post('/api/drafts', { draftFromOrganization: 'A' }), error => error.name === 'AbortError')
  await assert.rejects(b.api.switchPlatformContext(targetIdentity.organizationId, targetIdentity.membershipId), error => error.name === 'AbortError')
  release.resolve(); await switchB
  assert.equal(oldPageSends, 0); assert.equal(read(origin).token, 'B')
})

test('delayed events cannot let an old UI submit or log out using a completed replacement login', async () => {
  const origin = sharedOrigin({ events: false }); let oldPageSends = 0
  const a = await origin.page(async () => json(200, {})); await a.state().setSession('A', 'psr_A', undefined, identity())
  const b = await origin.page(async () => { oldPageSends++; return json(200, {}) })
  await a.state().setSession('B', 'psr_B', undefined, identity())
  await assert.rejects(b.api.api.post('/api/drafts', { draftFromOrganization: 'A' }), error => error.name === 'AbortError')
  assert.equal(oldPageSends, 0); assert.equal(read(origin).token, 'B')
  // Reset the page's observed A view to model another stale callback before any
  // storage event, then invoke the default logout (without an explicit fence).
  b.authStore.setState({ sessionId: 'stale-A-ui', token: 'A', sessionStatus: 'active' })
  assert.equal(await b.state().logout(), false); assert.equal(read(origin).token, 'B')
})

test('a logout during initial reconciliation prevents sending the original body without a bearer', async () => {
  const f = await fixture(async () => { assert.fail('old body must not be sent after logout') })
  const pending = f.api.api.post('/api/drafts', { draftFromOrganization: 'A' })
  await f.state().logout()
  await assert.rejects(pending, error => error.name === 'AbortError')
  assert.equal(f.calls.length, 0)
})

test('same-account same-organization membership and generation changes retire old requests across pages', async () => {
  for (const patch of [{ membershipId: targetIdentity.membershipId }, { identityVersion: '2' }, { accountVersion: '1' }]) {
    const origin = sharedOrigin({ events: false }), started = deferred(), release = deferred()
    const a = await origin.page(async () => { started.resolve(); await release.promise; return json(200, { private: 'old-member' }) })
    await a.state().setSession('member-A', 'psr_A', undefined, identity())
    const stale = a.captureSessionFence(), oldId = a.state().sessionId
    const pending = a.api.api.get('/api/old-member'); await started.promise
    const replacement = identity(patch), record = read(origin)
    // Keep the logical session ID deliberately equal: the complete member key
    // itself must fence a suspended tab, even before storage events arrive.
    origin.storage.set(key, JSON.stringify({ ...record, version: record.version + 1, identity: replacement, contextKey: contextIdentityKey(replacement), token: 'member-B', refreshToken: 'psr_B' }))
    release.resolve(); await assert.rejects(pending, error => error.name === 'AbortError')
    assert.equal(a.state().sessionId, oldId)
    assert.equal(a.state().contextKey, contextIdentityKey(replacement))
    assert.equal(await a.state().logout(stale), false)
    assert.equal(await a.state().setSession('late-A', 'psr_late-A', stale, identity()), false)
    assert.equal(read(origin).token, 'member-B')
  }
})

test('context selection submits both exact IDs and commits the complete server-proven namespace', async () => {
  const selected = identity({ membershipId: targetIdentity.membershipId, identityVersion: '9007199254740993', accountVersion: '4' })
  let submitted
  const f = await fixture(async (url, options) => {
    assert.equal(url, '/api/auth/context')
    submitted = JSON.parse(options.body)
    return json(200, contextResponse('selected-member', 'psr_selected-member', selected))
  })
  const previous = f.state().contextKey
  await f.api.switchPlatformContext(selected.organizationId, selected.membershipId)
  assert.deepEqual(submitted, { organizationId: selected.organizationId, membershipId: selected.membershipId })
  assert.deepEqual(f.record().identity, selected)
  assert.equal(f.record().contextKey, contextIdentityKey(selected))
  assert.notEqual(f.record().contextKey, previous)
})

for (const invalid of ['missing', 'customer', 'numeric-version', 'wrong-target']) test(`successful context with ${invalid} proof is quarantined without installing its tokens`, async () => {
  const proof = { ...targetIdentity }
  if (invalid === 'customer') proof.memberType = 'CUSTOMER'
  if (invalid === 'numeric-version') proof.identityVersion = 1
  if (invalid === 'wrong-target') proof.membershipId = identity().membershipId
  const response = contextResponse('untrusted-success', 'psr_untrusted-success', proof)
  if (invalid === 'missing') delete response.membershipId
  const f = await fixture(async () => json(200, response))
  await assert.rejects(f.api.switchPlatformContext(targetIdentity.organizationId, targetIdentity.membershipId), error => error.disposition === 'unknown')
  assert.equal(f.record().status, 'unknown')
  assert.equal(f.record().token, 'expired-access')
  assert.equal(f.record().refreshToken, 'psr_test-refresh')
  assert.equal(f.state().token, null)
  const calls = f.calls.length
  await assert.rejects(f.api.api.get('/api/private'), error => error.code === 'SESSION_UNKNOWN')
  assert.equal(f.calls.length, calls)
})

test('an identity-changing refresh result cannot be committed by the common lock owner', async () => {
  const f = await fixture(async () => assert.fail('no network needed for coordinator ownership contract'))
  const coordinator = f.load('src/lib/session-coordinator.ts').sessionCoordinator
  await assert.rejects(coordinator.operation('refresh', f.captureSessionFence(), async () => ({ value: 'wrong', tokens: {
    token: 'wrong-member-token', refreshToken: 'psr_wrong', identity: identity({ membershipId: targetIdentity.membershipId }),
  } })), error => error.disposition === 'unknown')
  assert.equal(f.record().status, 'unknown')
  assert.equal(f.record().token, 'expired-access')
  assert.equal(f.record().identity.membershipId, identity().membershipId)
})
