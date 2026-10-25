import assert from 'node:assert/strict'
import test from 'node:test'
import { deferred, fixture, identity, targetIdentity, json } from './helpers/session-fixture.mjs'

const sid = '019d0000-0000-7000-8000-000000000010'
const csrf = 'a'.repeat(64)
const choice = proof => ({ ...proof, status: 'ENABLED', consoleSelectable: true, selectionDisabledReason: null,
  tenantName: 'Owned tenant', organizationName: 'Owned organization', organizationPath: 'Owned organization', departmentId: null, departmentName: null })
const view = (proof, version = '1') => ({ sessionId: sid, version, csrfToken: csrf,
  account: { id: proof.userId, username: 'owned-bff-account', realName: 'Owned account', avatar: '' },
  memberships: [choice(identity()), choice(targetIdentity)], context: proof, expiresAt: new Date(Date.now() + 600_000).toISOString() })
const bridge = (token, proof, version = '1') => ({ accessToken: token, expiresIn: 300, sessionId: sid, version,
  membershipId: proof.membershipId, memberType: 'EMPLOYEE', identityVersion: proof.identityVersion, accountVersion: proof.accountVersion,
  user: { id: proof.userId, username: 'owned-bff-account', realName: 'Owned account', avatar: '', email: '', phone: '', gender: '', isSuperAdmin: false },
  tenant: { id: proof.tenantId, name: 'Owned tenant', code: 'owned', isPlatform: false },
  organization: { id: proof.organizationId, name: 'Owned organization', path: 'Owned organization' }, platformContext: false })

test('the real API renews BFF Cookie credentials after 401 without a browser refresh pair', async t => {
  let issued = 0, requests = 0
  const f = await fixture(async (url, options) => {
    if (url === '/auth/session') return json(200, view(identity()))
    if (url === '/auth/context-token') {
      assert.equal(options.credentials, 'same-origin')
      assert.equal(options.headers['X-CSRF-Token'], csrf)
      assert.equal(options.headers.Authorization, undefined)
      assert.deepEqual(JSON.parse(options.body), { expectedVersion: '1' })
      return json(200, bridge(++issued === 1 ? 'owned-initial-bridge' : 'owned-renewed-bridge', identity()))
    }
    if (url === '/api/owned-read') {
      requests++
      assert.equal(options.headers.Authorization, requests === 1 ? 'Bearer owned-initial-bridge' : 'Bearer owned-renewed-bridge')
      return json(requests === 1 ? 401 : 200, requests === 1 ? {} : { complete: true })
    }
    throw new Error('Unexpected endpoint; BFF must never fall back to legacy credentials')
  }, { authMode: 'BFF', session: false })
  t.after(() => f.load('src/lib/bff-session.ts').bffSessionCoordinator.dispose())
  const epoch = f.state().contextEpoch, logical = f.state().sessionId
  assert.equal((await f.api.api.get('/api/owned-read')).complete, true)
  assert.equal(issued, 2)
  assert.equal(requests, 2)
  assert.equal(f.state().authMode, 'BFF')
  assert.equal(f.state().refreshToken, null)
  assert.equal(f.state().contextEpoch, epoch)
  assert.equal(f.state().sessionId, logical)
  assert.equal(f.storage.has('platform-session-v2'), false)
  assert.equal(f.storage.has('platform-refresh-token'), false)
  assert.equal(JSON.stringify(f.state().bffSession).includes('csrfToken'), false)
  await assert.rejects(f.state().setSession('untrusted-browser-token', 'untrusted-browser-refresh', undefined, identity()), /不接受浏览器保存/)
})

test('a BFF member switch retires an actual API response even when transport ignores abort', async t => {
  const entered = deferred(), release = deferred()
  let current = identity(), revision = '1'
  const f = await fixture(async (url, options) => {
    if (url === '/auth/session') return json(200, view(current, revision))
    if (url === '/auth/context-token') {
      const body = JSON.parse(options.body)
      if (body.membershipId) {
        assert.equal(body.expectedVersion, '1')
        assert.equal(body.membershipId, targetIdentity.membershipId)
        assert.equal(body.organizationId, targetIdentity.organizationId)
        current = targetIdentity; revision = '2'
      }
      return json(200, bridge(current.membershipId === targetIdentity.membershipId ? 'owned-member-b' : 'owned-member-a', current, revision))
    }
    if (url === '/api/held-owned-response') { entered.resolve(); await release.promise; return json(200, { retiredData: true }) }
    throw new Error('Unexpected endpoint')
  }, { authMode: 'BFF', session: false })
  t.after(() => f.load('src/lib/bff-session.ts').bffSessionCoordinator.dispose())
  const old = f.captureSessionFence(), epoch = f.state().contextEpoch
  const pending = f.api.api.get('/api/held-owned-response')
  const rejected = assert.rejects(pending, error => error.name === 'AbortError')
  await entered.promise
  await f.api.switchPlatformContext(targetIdentity.organizationId, targetIdentity.membershipId, old)
  release.resolve()
  await rejected
  assert.equal(f.state().identity.membershipId, targetIdentity.membershipId)
  assert.ok(f.state().contextEpoch > epoch)
  assert.notEqual(f.state().sessionId, old.sessionId)
  assert.equal(f.state().refreshToken, null)
  assert.equal(f.storage.has('platform-session-v2'), false)
})

test('BFF authority 503 pauses the real API instead of invoking local password or platform refresh', async t => {
  let outage = false, businessCalls = 0
  const f = await fixture(async url => {
    if (url === '/auth/session') return outage ? json(503, {}) : json(200, view(identity()))
    if (url === '/auth/context-token') return json(200, bridge('owned-memory-bridge', identity()))
    if (url === '/api/must-remain-paused') { businessCalls++; return json(200, {}) }
    throw new Error('Unexpected legacy fallback')
  }, { authMode: 'BFF', session: false })
  t.after(() => f.load('src/lib/bff-session.ts').bffSessionCoordinator.dispose())
  outage = true
  await assert.rejects(f.api.api.get('/api/must-remain-paused'))
  assert.equal(businessCalls, 0)
  assert.equal(f.state().token, null)
  assert.equal(f.state().refreshToken, null)
  assert.equal(f.state().sessionStatus, 'unknown')
  assert.equal(f.state().bffSession?.sessionId, sid, 'Recovery retains the account view so explicit reauthentication must remain available')
  assert.equal(f.calls.some(call => ['/oauth/token', '/api/auth/login', '/api/auth/context'].includes(call.url)), false)
})
