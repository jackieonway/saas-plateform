import assert from 'node:assert/strict'
import test from 'node:test'
import { createNotificationContextStore, runNotificationRequest } from '../src/lib/notification-context.ts'
import { isNotificationRecord, isNotificationSocketContext, isRealtimeNotification, notificationLink } from '../src/lib/notification-protocol.ts'
import { contextIdentityKey } from '../src/lib/context-identity.ts'

const deferred = () => { let resolve, reject; const promise = new Promise((a, b) => { resolve = a; reject = b }); return { promise, resolve, reject } }
const id = (value) => `01962345-6789-7abc-8def-${value.toString(16).padStart(12, '0')}`
const proof = { userId: id(1), tenantId: id(2), organizationId: id(3), membershipId: id(4), memberType: 'EMPLOYEE', identityVersion: '1', accountVersion: '0' }
const profile = (identity) => ({ user: { id: identity.userId }, tenant: { id: identity.tenantId }, organization: { id: identity.organizationId },
  membershipId: identity.membershipId, memberType: identity.memberType, identityVersion: identity.identityVersion, accountVersion: identity.accountVersion })
function auth() {
  let state = { token: 'mock-session-A', contextEpoch: 1, contextSwitching: false,
    identity: { ...proof }, contextKey: contextIdentityKey(proof), profile: profile(proof) }
  const listeners = new Set()
  const store = createNotificationContextStore(() => state, (listener) => { listeners.add(listener); return () => listeners.delete(listener) })
  return { store, listeners, state: () => state, set: (patch) => { state = { ...state, ...patch }; for (const listener of [...listeners]) listener() } }
}
const replaceIdentity = (a, patch, statePatch = {}) => {
  const identity = { ...a.state().identity, ...patch }
  a.set({ identity, contextKey: contextIdentityKey(identity), profile: profile(identity), ...statePatch })
}
const options = (scope) => ({ bearer: scope.token, signal: scope.signal, current: scope.current })
const identity = { userId: proof.userId, tenantId: proof.tenantId, organizationId: proof.organizationId }
const notification = (extra = {}) => ({ contractVersion: '1.0', id: id(10), ...identity,
  applicationId: id(11), publicationId: id(12), type: 'INFO', title: 'Current notification',
  content: 'Body', link: '/micro/orders/1', createdAt: new Date().toISOString(), ...extra })
const record = (extra = {}) => ({ ...notification(), sourceKind: 'APP', applicationName: 'Orders', isRead: false, readAt: null, ...extra })

test('begin organization switch synchronously invalidates old lifetime before a replacement token exists', () => {
  const a = auth(), captured = a.store.getSnapshot()
  let synchronousAbort = false
  captured.signal.addEventListener('abort', () => { synchronousAbort = true })
  a.set({ contextSwitching: true, contextEpoch: 2, profile: null })
  assert.equal(synchronousAbort, true)
  assert.equal(captured.current(), false)
  assert.equal(a.store.getSnapshot(), null)
  a.store.dispose(); assert.equal(a.listeners.size, 0)
})

test('cache identity includes the complete member proof and epoch and survives profile content updates', () => {
  const a = auth(), first = a.store.getSnapshot()
  assert.deepEqual(first.key, ['notifications', proof.userId, proof.tenantId, proof.organizationId,
    proof.membershipId, proof.memberType, proof.identityVersion, proof.accountVersion, 1])
  a.set({ profile: { ...a.state().profile, applications: ['changed-entry'] } })
  assert.equal(a.store.getSnapshot(), first)
  for (const [field, value] of [['userId', id(20)], ['tenantId', id(21)], ['organizationId', id(22)]]) {
    const prior = a.store.getSnapshot()
    replaceIdentity(a, { [field]: value })
    assert.equal(prior.signal.aborted, true)
    assert.equal(prior.current(), false)
    assert.notDeepEqual(a.store.getSnapshot().key, prior.key)
  }
  a.store.dispose()
})

test('a token replacement also cancels a captured request even if caller forgot to advance epoch', () => {
  const a = auth(), captured = a.store.getSnapshot()
  a.set({ token: 'mock-session-B' })
  assert.equal(captured.signal.aborted, true)
  assert.equal(captured.current(), false)
  assert.equal(a.store.getSnapshot().token, 'mock-session-B')
  a.store.dispose()
})

test('old HTTP body and old HTTP errors cannot cross into a new organization when transport ignores AbortSignal', async () => {
  for (const rejected of [false, true]) {
    const a = auth(), captured = a.store.getSnapshot(), response = deferred()
    const pending = runNotificationRequest(options(captured), () => response.promise)
    replaceIdentity(a, { organizationId: id(22) }, { token: 'mock-session-B', contextEpoch: 2 })
    if (rejected) response.reject(new Error('old organization HTTP error'))
    else response.resolve({ count: 999, title: 'old organization body' })
    await assert.rejects(pending, (error) => error.name === 'AbortError')
    assert.equal(a.store.getSnapshot().organizationId, id(22))
    a.store.dispose()
  }
})

test('cancelled queries do not issue HTTP or mutate captured data', async () => {
  const a = auth(), scope = a.store.getSnapshot(), caller = new AbortController()
  caller.abort()
  let requested = false
  await assert.rejects(runNotificationRequest({ ...options(scope), signal: AbortSignal.any([scope.signal, caller.signal]) },
    async () => { requested = true }), (error) => error.name === 'AbortError')
  assert.equal(requested, false)
  a.store.dispose()
})

test('authenticated acknowledgement rejects stale ownership, missing profile fields, wrong version and expiry', () => {
  const valid = { contractVersion: '1.0', ...identity, expiresAt: new Date(Date.now() + 60_000).toISOString() }
  assert.equal(isNotificationSocketContext(valid, identity), true)
  // Public ack intentionally contains no internal sessionId or token.
  for (const patch of [{ userId: 'other' }, { tenantId: 'other' }, { organizationId: 'other' },
    { organizationId: undefined }, { contractVersion: '2.0' }, { expiresAt: 'invalid' }, { expiresAt: new Date(0).toISOString() }]) {
    assert.equal(isNotificationSocketContext({ ...valid, ...patch }, identity), false)
  }
})

test('realtime DTO requires complete recipient ownership and one coherent application publication identity', () => {
  assert.equal(isRealtimeNotification(notification(), identity), true)
  assert.equal(isRealtimeNotification(notification({ applicationId: null, publicationId: null }), identity), true)
  for (const patch of [{ userId: 'other' }, { tenantId: 'other' }, { organizationId: 'other' }, { contractVersion: '0.9' },
    { applicationId: null }, { publicationId: null }, { id: '' }, { title: '' }, { type: 'UNKNOWN' },
    { content: null }, { createdAt: 'invalid' }, { title: 'x'.repeat(201) }]) {
    assert.equal(isRealtimeNotification(notification(patch), identity), false)
  }
})

test('HTTP inbox rows reject legacy/unowned rows and keep APP/SYSTEM source contract distinct', () => {
  assert.equal(isNotificationRecord(record()), true)
  assert.equal(isNotificationRecord(record({ sourceKind: 'SYSTEM', applicationId: null, publicationId: null, applicationName: null })), true)
  for (const patch of [{ sourceKind: 'LEGACY' }, { tenantId: null }, { userId: undefined }, { isRead: undefined },
    { applicationName: undefined }, { readAt: 'bad' }, { sourceKind: 'SYSTEM', applicationId: 'app' }]) {
    assert.equal(isNotificationRecord(record(patch)), false)
  }
})

test('notification toast actions only accept normalized host routes', () => {
  const origin = 'https://platform.test'
  assert.equal(notificationLink('/micro/orders/1?view=detail#top', origin), '/micro/orders/1?view=detail#top')
  for (const link of ['https://evil.test', 'javascript:alert(1)', '//evil.test', '/\\evil.test', '/%2f%2fevil.test', '/%5cevil.test', '/%00evil']) {
    assert.equal(notificationLink(link, origin), null)
  }
})

test('same-organization member, type, identity version and account version changes synchronously retire scopes', () => {
  for (const patch of [{ membershipId: id(40) }, { memberType: 'CUSTOMER' }, { identityVersion: '2' }, { accountVersion: '1' }]) {
    const a = auth(), captured = a.store.getSnapshot()
    let aborted = false
    captured.signal.addEventListener('abort', () => { aborted = true })
    replaceIdentity(a, patch)
    const replacement = a.store.getSnapshot()
    assert.equal(aborted, true)
    assert.equal(captured.current(), false)
    assert.equal(replacement.current(), true)
    assert.equal(replacement.userId, captured.userId)
    assert.equal(replacement.organizationId, captured.organizationId)
    assert.notDeepEqual(replacement.key, captured.key)
    assert.notEqual(replacement.namespace, captured.namespace)
    a.store.dispose()
  }
})

test('notification scope requires matching bound and profile proofs without legacy identity fallback', () => {
  for (const patch of [state => ({ identity: null }), state => ({ contextKey: null }),
    state => ({ contextKey: JSON.stringify([state.identity.userId, state.identity.organizationId]) }),
    state => ({ profile: { user: state.profile.user, tenant: state.profile.tenant, organization: state.profile.organization } }),
    state => ({ profile: { ...state.profile, membershipId: id(40) } }),
    state => ({ profile: { ...state.profile, memberType: 'CUSTOMER' } }),
    state => ({ profile: { ...state.profile, identityVersion: '2' } }),
    state => ({ profile: { ...state.profile, accountVersion: '1' } }),
    state => ({ profile: { ...state.profile, identityVersion: 1 } }),
    state => ({ profile: { ...state.profile, identityVersion: '01' } }),
    state => ({ profile: { ...state.profile, accountVersion: '00' } }),
    state => ({ profile: { ...state.profile, identityVersion: '9223372036854775808' } })]) {
    const a = auth(), captured = a.store.getSnapshot()
    a.set(patch(a.state()))
    assert.equal(captured.signal.aborted, true)
    assert.equal(captured.current(), false)
    assert.equal(a.store.getSnapshot(), null)
    a.store.dispose()
  }
})

test('late notification bodies and errors cannot cross a same-user same-org membership generation', async () => {
  for (const patch of [{ membershipId: id(40) }, { memberType: 'CUSTOMER' }, { identityVersion: '2' }, { accountVersion: '1' }]) {
    for (const rejected of [false, true]) {
      const a = auth(), captured = a.store.getSnapshot(), response = deferred()
      const pending = runNotificationRequest(options(captured), () => response.promise)
      replaceIdentity(a, patch)
      if (rejected) response.reject(new Error('late previous-member failure'))
      else response.resolve({ count: 999, title: 'late previous-member inbox' })
      await assert.rejects(pending, error => error.name === 'AbortError')
      assert.equal(a.store.getSnapshot().current(), true)
      a.store.dispose()
    }
  }
})

test('public ack and event remain five and twelve fields while callbacks use captured member lifetime', () => {
  const a = auth(), captured = a.store.getSnapshot()
  const ack = { contractVersion: '1.0', ...identity, expiresAt: new Date(Date.now() + 60_000).toISOString() }
  const event = notification()
  assert.equal(Object.keys(ack).length, 5)
  assert.equal(Object.keys(event).length, 12)
  assert.equal(isNotificationSocketContext(ack, captured), true)
  assert.equal(isRealtimeNotification(event, captured), true)
  replaceIdentity(a, { membershipId: id(40), identityVersion: '9007199254740993', accountVersion: '9223372036854775807' })
  assert.equal(captured.current(), false)
  // The public fields intentionally cannot distinguish the two memberships;
  // the captured scope must reject the old callback before consuming either.
  assert.equal(isNotificationSocketContext(ack, a.store.getSnapshot()), true)
  assert.equal(isRealtimeNotification(event, a.store.getSnapshot()), true)
  assert.equal(a.store.getSnapshot().identityVersion, '9007199254740993')
  assert.equal(a.store.getSnapshot().accountVersion, '9223372036854775807')
  a.store.dispose()
})

test('a profile mutation without an auth notification still invalidates captured callbacks and requests', async () => {
  const a = auth(), captured = a.store.getSnapshot()
  a.state().profile.identityVersion = '2'
  assert.equal(captured.current(), false)
  let requested = false
  await assert.rejects(runNotificationRequest(options(captured), async () => { requested = true }), error => error.name === 'AbortError')
  assert.equal(requested, false)
  a.store.dispose()
})
