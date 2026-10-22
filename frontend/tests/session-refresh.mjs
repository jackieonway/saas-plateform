import assert from 'node:assert/strict'
import test from 'node:test'
import { fixture, deferred, json } from './helpers/session-fixture.mjs'

test('concurrent expired requests share one refresh, retry with rotated token and preserve context', async () => {
  const ready = deferred(), release = deferred(); let refreshes = 0
  const f = await fixture(async (url, options) => {
    if (url === '/oauth/token') { refreshes++; ready.resolve(); await release.promise; assert.equal(new URLSearchParams(options.body).get('grant_type'), 'refresh_token'); return json(200, { access_token: 'renewed-access', refresh_token: 'psr_rotated', expires_in: 3600 }) }
    return options.headers.Authorization === 'Bearer renewed-access' ? json(200, { ok: true }) : json(401, { message: 'expired' })
  })
  f.state().setCurrentApplication('authorized-app')
  const epoch = f.state().contextEpoch
  const first = f.api.api.get('/api/users'), second = f.api.api.get('/api/roles')
  await ready.promise; release.resolve()
  assert.equal((await first).ok, true); assert.equal((await second).ok, true)
  assert.equal(refreshes, 1); assert.equal(f.state().contextEpoch, epoch)
  assert.equal(f.state().currentApplicationId, 'authorized-app')
  assert.equal(f.record().refreshToken, 'psr_rotated'); assert.equal(f.storage.has('platform-refresh-token'), false)
  assert.equal(f.redirects.length, 0)
})

test('retry occurs only once and a still-invalid session clears all credentials', async () => {
  let refreshes = 0
  const f = await fixture(async url => url === '/oauth/token' ? (refreshes++, json(200, { access_token: 'new', refresh_token: 'psr_new' })) : json(401, { message: 'revoked' }))
  await assert.rejects(f.api.api.get('/api/users'), error => error.status === 401)
  assert.equal(refreshes, 1); assert.equal(f.state().token, null)
  assert.equal(f.record().refreshToken, null); assert.equal(f.record().status, 'anonymous'); assert.equal(f.redirects.length, 1)
})

test('late refresh from previous tenant cannot overwrite or log out a new session', async () => {
  const started = deferred(), release = deferred()
  const f = await fixture(async url => { if (url === '/oauth/token') { started.resolve(); await release.promise; return json(200, { access_token: 'old-rotated', refresh_token: 'psr_old' }) } return json(401, {}) })
  const rejected = assert.rejects(f.api.api.get('/api/users'), error => error.name === 'AbortError')
  await started.promise
  await f.state().setSession('tenant-B-access', 'psr_tenant-B')
  release.resolve(); await rejected
  assert.equal(f.state().token, 'tenant-B-access'); assert.equal(f.redirects.length, 0)
  assert.equal(f.record().refreshToken, 'psr_tenant-B')
})

test('login credential tickets never trigger platform-session refresh', async () => {
  const f = await fixture(async () => json(401, { message: 'ticket expired' }))
  await assert.rejects(f.api.api.post('/api/auth/context', {}, { bearer: 'credential-ticket', suppress401: true }), error => error.status === 401)
  assert.equal(f.calls.length, 1); assert.equal(f.state().token, 'expired-access'); assert.equal(f.redirects.length, 0)
})

test('late successful data response after context switch is discarded', async () => {
  const started = deferred(), release = deferred()
  const f = await fixture(async () => { started.resolve(); await release.promise; return json(200, { tenantData: 'old' }) })
  const pending = f.api.api.get('/api/tenants'); await started.promise
  f.state().beginContextSwitch(); release.resolve()
  await assert.rejects(pending, error => error.name === 'AbortError')
})

// The old assertions allowed an unsafe retry of a potentially consumed refresh.
// Preserve the durable credentials, but quarantine requests until a new login.
for (const failure of ['network', 'server']) test(`uncertain refresh ${failure} failure preserves credentials and prevents replay`, async () => {
  let refreshes = 0
  const f = await fixture(async url => { if (url === '/oauth/token') { refreshes++; if (failure === 'network') throw new Error('offline'); return json(503, {}) } return json(401, {}) })
  await assert.rejects(f.api.api.get('/api/users'), error => error.code === 'SESSION_UNKNOWN')
  assert.equal(f.state().token, null); assert.equal(f.record().token, 'expired-access')
  assert.equal(f.record().refreshToken, 'psr_test-refresh'); assert.equal(f.record().status, 'unknown')
  assert.equal(f.redirects.length, 0)
  await assert.rejects(f.api.api.get('/api/users'), error => error.code === 'SESSION_UNKNOWN')
  assert.equal(refreshes, 1)
})

test('expired access refreshes inside a context operation without recursively taking the lock', async () => {
  let switchingDuringRefresh = false
  const f = await fixture(async (url, options) => {
    if (url === '/oauth/token') { switchingDuringRefresh = f.state().contextSwitching; return json(200, { access_token: 'renewed', refresh_token: 'psr_new' }) }
    return options.headers.Authorization === 'Bearer renewed' ? json(200, { accessToken: 'selected-org', refreshToken: 'psr_selected' }) : json(401, {})
  })
  const result = await f.api.switchPlatformContext('other-org')
  assert.equal(result.accessToken, 'selected-org'); assert.equal(switchingDuringRefresh, true)
  assert.equal(f.state().contextSwitching, false); assert.equal(f.state().token, 'selected-org')
  assert.equal(f.record().refreshToken, 'psr_selected')
})

test('invalid refresh clears credentials and legacy setToken removes old refresh/app state', async () => {
  const f = await fixture(async url => json(url === '/oauth/token' ? 400 : 401, {}))
  await assert.rejects(f.api.api.get('/api/users'), error => error.code === 'SESSION_REFRESH_FAILED')
  assert.equal(f.state().token, null); assert.equal(f.state().refreshToken, null)
  await f.state().setSession('first', 'psr_first'); f.state().setCurrentApplication('app')
  await f.state().setToken('legacy')
  assert.equal(f.state().refreshToken, null); assert.equal(f.state().currentApplicationId, null)
  assert.equal(f.record().token, 'legacy'); assert.equal(f.storage.has('platform-token'), false)
})

test('logout renews an expired access token under the same lock before revoking the remote session', async () => {
  const events = []
  let revoked = false
  const f = await fixture(async (url, options) => {
    if (url === '/oauth/token') {
      events.push('refresh')
      return json(200, { access_token: 'renewed-for-logout', refresh_token: 'psr_logout' })
    }
    assert.equal(url, '/api/auth/logout')
    events.push(options.headers.Authorization)
    revoked = options.headers.Authorization === 'Bearer renewed-for-logout'
    return json(revoked ? 200 : 401, {})
  })
  assert.equal(await f.api.logoutPlatformSession(), true)
  assert.equal(revoked, true)
  assert.deepEqual(events, ['Bearer expired-access', 'refresh', 'Bearer renewed-for-logout'])
  assert.equal(f.record().status, 'anonymous')
  assert.equal(f.record().refreshToken, null)
  assert.equal(f.redirects.length, 0)
})

test('logout clears a definitely invalid refresh but quarantines an uncertain refresh result', async () => {
  for (const status of [400, 503]) {
    let refreshes = 0
    const f = await fixture(async url => {
      if (url === '/oauth/token') { refreshes++; return json(status, {}) }
      return json(401, {})
    })
    if (status === 400) {
      assert.equal(await f.api.logoutPlatformSession(), true)
      assert.equal(f.record().status, 'anonymous')
    } else {
      await assert.rejects(f.api.logoutPlatformSession(), error => error.disposition === 'unknown')
      assert.equal(f.record().status, 'unknown')
      assert.equal(f.record().refreshToken, 'psr_test-refresh')
    }
    assert.equal(refreshes, 1)
    assert.equal(f.calls.length, 2)
  }
})

test('a delayed logout refresh cannot revoke or clear a replacement login', async () => {
  const started = deferred(), release = deferred()
  const f = await fixture(async url => {
    if (url === '/oauth/token') {
      started.resolve()
      await release.promise
      return json(200, { access_token: 'old-renewed', refresh_token: 'psr_old-renewed' })
    }
    return json(401, {})
  })
  const leaving = f.api.logoutPlatformSession()
  await started.promise
  await f.state().setSession('replacement-access', 'psr_replacement')
  release.resolve()
  assert.equal(await leaving, false)
  assert.equal(f.record().token, 'replacement-access')
  assert.equal(f.record().refreshToken, 'psr_replacement')
  assert.equal(f.calls.filter(call => call.url === '/api/auth/logout').length, 1)
})
