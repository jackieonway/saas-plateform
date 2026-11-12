import assert from 'node:assert/strict'
import test from 'node:test'
import { componentFixture } from './helpers/component-fixture.mjs'

const uuid = '01962345-6789-7abc-8def-000000000001'
const fixture = componentFixture('src/pages/system/oauth/UnifiedClientsPage.tsx', ['clientFormSchema', 'clientTypeFor', 'tokenMethodFor'], {
  './ClientAuthorizationSettings': { ClientAuthorizationSettings: 'ClientAuthorizationSettings' },
  './client-policy': { validTokenSeconds: value => value === '' || /^\d+$/.test(value) && Number(value)>=60 && Number(value)<=604800 },
  '@/lib/uuid': { isUuidV7: value => typeof value === 'string' && /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(value) },
})
const { clientFormSchema, clientTypeFor, tokenMethodFor } = fixture.exports
class FixtureApiError extends Error {
  constructor(message, code, status, requestId, operationId) { super(message); Object.assign(this, { code, status, requestId, operationId }) }
}
const user = (overrides = {}) => ({
  applicationId: uuid,
  environment: 'test',
  displayName: 'CRM Web',
  subjectMode: 'USER',
  runtimeKind: 'WEB_BFF',
  contextMode: 'ACCOUNT_ONLY',
  allowedMemberTypes: [],
  refreshToken: false,
  autoAuthorize: false, accessTokenTtlSeconds: '', refreshTokenTtlSeconds: '', clientTokenTtlSeconds: '',
  redirectUrisText: 'https://client.example.test/callback',
  postLogoutRedirectUrisText: '',
  logoutMode: 'SHORT_SESSION_ONLINE_CHECK',
  backchannelLogoutUri: '',
  backchannelTargetId: '',
  status: 'DISABLED',
  consentGroupId: '',
  permissions: [{ resourceKey: 'userinfo', scopesText: 'openid, profile' }],
  ...overrides,
})

test('unified client form accepts supported USER, browser, native and MACHINE contracts', () => {
  assert.equal(clientFormSchema.safeParse(user()).success, true)
  assert.equal(clientFormSchema.safeParse(user({ runtimeKind: 'SPA' })).success, true)
  assert.equal(clientFormSchema.safeParse(user({ runtimeKind: 'NATIVE' })).success, true)
  assert.equal(clientTypeFor('USER', 'WEB_BFF'), 'CONFIDENTIAL')
  assert.equal(clientTypeFor('USER', 'SPA'), 'PUBLIC')
  assert.equal(clientTypeFor('USER', 'NATIVE'), 'PUBLIC')
  assert.equal(tokenMethodFor('USER', 'WEB_BFF'), 'client_secret_basic')
  assert.equal(tokenMethodFor('USER', 'SPA'), 'none')
  const machine = user({
    subjectMode: 'MACHINE', runtimeKind: 'SERVICE', contextMode: 'NONE', refreshToken: false,
    redirectUrisText: '', postLogoutRedirectUrisText: '',
    permissions: [{ resourceKey: 'sync-api', scopesText: 'records.read' }],
  })
  assert.equal(clientFormSchema.safeParse(machine).success, true)
  assert.equal(clientTypeFor('MACHINE', 'SERVICE'), 'CONFIDENTIAL')
  assert.equal(tokenMethodFor('MACHINE', 'SERVICE'), 'client_secret_basic')
})

test('back-channel form requires a confidential user, exact HTTPS endpoint and an owner reference', () => {
  const registered = { logoutMode: 'BACK_CHANNEL', backchannelLogoutUri: 'https://client.example.test/auth/backchannel-logout', backchannelTargetId: uuid }
  assert.equal(clientFormSchema.safeParse(user(registered)).success, true)
  for (const patch of [{ runtimeKind: 'SPA' }, { subjectMode: 'MACHINE' }, { backchannelTargetId: '' }, { backchannelLogoutUri: 'https://client.example.test/logout?token=bad' }, { backchannelLogoutUri: 'http://client.example.test/logout' }, { logoutMode: 'SHORT_SESSION_ONLINE_CHECK' }]) assert.equal(clientFormSchema.safeParse(user({ ...registered, ...patch })).success, false)
})

test('unified client form rejects illegal subject/runtime/context combinations', () => {
  assert.equal(clientFormSchema.safeParse(user({ runtimeKind: 'SERVICE' })).success, false)
  assert.equal(clientFormSchema.safeParse(user({ contextMode: 'PLATFORM_CONTEXT' })).success, false)
  assert.equal(clientFormSchema.safeParse(user({ contextMode: 'PLATFORM_CONTEXT', allowedMemberTypes: ['EMPLOYEE', 'CUSTOMER'] })).success, false)
  for (const runtimeKind of ['SPA', 'NATIVE']) {
    assert.equal(clientFormSchema.safeParse(user({ runtimeKind, contextMode: 'PLATFORM_CONTEXT', allowedMemberTypes: ['EMPLOYEE'] })).success, false)
  }
  assert.equal(clientFormSchema.safeParse(user({ contextMode: 'PLATFORM_CONTEXT', allowedMemberTypes: ['EMPLOYEE'] })).success, true)
  assert.equal(clientFormSchema.safeParse(user({ allowedMemberTypes: ['EMPLOYEE'] })).success, false)
  assert.equal(clientFormSchema.safeParse(user({ permissions: [{ resourceKey: 'userinfo', scopesText: 'profile' }] })).success, false)
  assert.equal(clientFormSchema.safeParse(user({
    subjectMode: 'MACHINE', runtimeKind: 'SERVICE', contextMode: 'ACCOUNT_ONLY',
    permissions: [{ resourceKey: 'sync-api', scopesText: 'records.read' }],
  })).success, false)
  assert.equal(clientFormSchema.safeParse(user({
    subjectMode: 'MACHINE', runtimeKind: 'SERVICE', contextMode: 'NONE', refreshToken: true,
    redirectUrisText: '', permissions: [{ resourceKey: 'sync-api', scopesText: 'records.read' }],
  })).success, false)
  assert.equal(clientFormSchema.safeParse(user({
    subjectMode: 'MACHINE', runtimeKind: 'SERVICE', contextMode: 'NONE',
    redirectUrisText: '', permissions: [{ resourceKey: 'userinfo', scopesText: 'openid' }],
  })).success, false)
})

test('unified client form enforces exact HTTPS callback and scope/resource syntax', () => {
  assert.equal(clientFormSchema.safeParse(user({ redirectUrisText: 'http://client.example.test/callback' })).success, false)
  assert.equal(clientFormSchema.safeParse(user({ redirectUrisText: 'https://*.example.test/callback' })).success, false)
  assert.equal(clientFormSchema.safeParse(user({ redirectUrisText: 'https://client.example.test/callback\nhttps://client.example.test/callback' })).success, false)
  assert.equal(clientFormSchema.safeParse(user({ permissions: [{ resourceKey: 'Bad Resource', scopesText: 'openid' }] })).success, false)
  assert.equal(clientFormSchema.safeParse(user({ permissions: [{ resourceKey: 'userinfo', scopesText: 'openid, openid' }] })).success, false)
})

test('identity writes keep their browser operation handle when the complete HTTP response is lost', async () => {
  const writes = [], operationId = '01962345-6789-7abc-8def-000000000002'
  const requests = componentFixture('src/api/identity-clients.ts', ['identityClientsApi'], {
    '@/lib/uuid': { newUuid: () => operationId },
    '@/lib/identity-operation-recovery': { beginIdentityOperation: () => ({ operationId, contextKey: 'owned-context' }), finishIdentityOperation() {} },
    '@/lib/api': { ApiError: FixtureApiError, qs: () => '', api: {
      post: async (path, body, options) => { writes.push({ path, body, options }); throw new FixtureApiError('connection lost', 'NETWORK_ERROR', 0) },
    } },
  }).exports.identityClientsApi
  await assert.rejects(requests.rotateCredential(uuid, 2), error =>
    error.code === 'OIDC_REMOTE_RESULT_UNKNOWN' && error.operationId === operationId)
  assert.deepEqual(writes, [{ path: `/api/identity/clients/${uuid}/credentials`, body: { expectedVersion: 2 }, options: { operationId } }])
})

test('identity writes preserve definite version conflicts rather than treating them as unknown commits', async () => {
  const conflict = new FixtureApiError('version changed', 'version_conflict', 409)
  const requests = componentFixture('src/api/identity-clients.ts', ['identityClientsApi'], {
    '@/lib/uuid': { newUuid: () => uuid },
    '@/lib/identity-operation-recovery': { beginIdentityOperation: () => ({ operationId: uuid, contextKey: 'owned-context' }), finishIdentityOperation() {} },
    '@/lib/api': { ApiError: FixtureApiError, qs: () => '', api: { patch: async () => { throw conflict } } },
  }).exports.identityClientsApi
  await assert.rejects(requests.update(uuid, {}), error => error === conflict)
})

function recoveryFixture() {
  const values = new Map()
  const storage = { getItem: key => values.get(key) ?? null, setItem: (key, value) => values.set(key, value), removeItem: key => values.delete(key) }
  let owner = { accountId: uuid, sessionId: 'owned-session:1', contextKey: JSON.stringify([uuid, 'member-generation-1']) }
  const factory = componentFixture('src/lib/identity-operation-recovery.ts', ['createIdentityOperationRecovery'], {
    '@/lib/api': { ApiError: FixtureApiError },
    '@/lib/uuid': { newUuid: () => '01962345-6789-7abc-8def-000000000003', isUuidV7: value => typeof value === 'string' && /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(value) },
    '@/stores/auth': { authStore: { getState: () => ({}) } },
  }).exports.createIdentityOperationRecovery
  const recovery = factory(() => storage, () => owner)
  return { values, storage, recovery, reload: () => factory(() => storage, () => owner), switchOwner: value => { owner = value }, owner: () => owner }
}

test('pending recovery survives reload using only whitelisted metadata and isolates account contexts', () => {
  const f = recoveryFixture()
  const pending = f.recovery.begin('create', 'Owned client')
  const encoded = [...f.values.values()][0]
  assert.deepEqual(Object.keys(JSON.parse(encoded)).sort(), ['accountId', 'action', 'clientLabel', 'contextKey', 'operationId', 'schema', 'sessionId'])
  assert.deepEqual(f.reload().read(), pending)
  assert.throws(() => f.reload().begin('create', 'Duplicate'), error => error.operationId === pending.operationId)
  const original = f.owner()
  f.switchOwner({ ...original, accountId: '01962345-6789-7abc-8def-000000000004', contextKey: 'different-proven-account-context' })
  assert.equal(f.reload().read(), null)
  f.switchOwner({ ...original, sessionId: 'different-authorized-session:1' })
  assert.equal(f.reload().read().sessionId, original.sessionId, 'Keep the original session for explicit server ownership rejection')
  assert.throws(() => f.reload().begin('rotate', 'Duplicate'), error => error.operationId === pending.operationId)
  f.recovery.finish('01962345-6789-7abc-8def-000000000005')
  assert.equal(f.values.size, 1, 'A different operation cannot discard pending recovery')
  f.recovery.finish(pending.operationId)
  assert.equal(f.values.size, 0)
})

test('identity API persists before dispatch, retains unknown and aborted results, and clears definite terminal results', async () => {
  const f = recoveryFixture()
  let fail = new FixtureApiError('lost entire HTTP response', 'NETWORK_ERROR', 0), writes = 0
  const requests = componentFixture('src/api/identity-clients.ts', ['identityClientsApi'], {
    '@/lib/identity-operation-recovery': { beginIdentityOperation: f.recovery.begin, finishIdentityOperation: f.recovery.finish },
    '@/lib/api': { ApiError: FixtureApiError, qs: () => '', api: { post: async (_path, _body, options) => {
      writes++
      assert.equal(f.reload().read().operationId, options.operationId, 'Persist the recovery handle before the HTTP mutation starts')
      if (fail) throw fail
      return { id: uuid }
    } } },
  }).exports.identityClientsApi
  const body = { displayName: 'Owned client' }
  await assert.rejects(requests.create(body), error => error.code === 'OIDC_REMOTE_RESULT_UNKNOWN')
  assert.equal(f.values.size, 1)
  await assert.rejects(requests.create(body), error => error.code === 'OIDC_REMOTE_RESULT_UNKNOWN')
  assert.equal(writes, 1)
  f.recovery.finish(f.recovery.read().operationId)
  fail = new DOMException('context changed after dispatch', 'AbortError')
  await assert.rejects(requests.create(body), error => error.name === 'AbortError')
  assert.equal(f.values.size, 1, 'Navigation or context abort cannot prove non-application')
  f.recovery.finish(f.recovery.read().operationId)
  fail = new FixtureApiError('definite version conflict', 'version_conflict', 409)
  await assert.rejects(requests.create(body), error => error.status === 409)
  assert.equal(f.values.size, 0)
  fail = null
  await requests.create(body)
  assert.equal(f.values.size, 0)
})

test('unavailable or malformed recovery storage blocks HTTP mutations', async () => {
  const f = recoveryFixture()
  f.storage.setItem = () => { throw new Error('quota unavailable') }
  assert.throws(() => f.recovery.begin('create', 'Owned client'), error => error.code === 'OPERATION_STORAGE_UNAVAILABLE')
  const other = recoveryFixture()
  const pending = other.recovery.begin('create', 'Owned client')
  other.values.set([...other.values.keys()][0], JSON.stringify({ ...pending, body: { secret: 'forbidden-field' } }))
  assert.equal(other.recovery.read(), null)
  assert.throws(() => other.recovery.begin('create', 'Duplicate'), error => error.code === 'OPERATION_STORAGE_INVALID')
})
