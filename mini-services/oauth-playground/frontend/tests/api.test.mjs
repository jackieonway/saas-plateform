import test from 'node:test'
import assert from 'node:assert/strict'
import { apiRequest, publicData } from '../src/api.js'

test('resource inspection redacts credential fields at every nesting level without hiding public grant metadata', () => {
  const input = { granted: true, tokenType: 'Bearer', hasRefreshToken: true, scope: 'profile', access_token: 'private', password: 'private', nested: [{ clientSecret: 'private', code_verifier: 'private', device_code: 'private', roles: ['reader'] }] }
  const data = publicData(input)
  assert.equal(data.access_token, '[已隐藏]')
  assert.equal(data.password, '[已隐藏]')
  assert.equal(data.nested[0].clientSecret, '[已隐藏]')
  assert.equal(data.nested[0].code_verifier, '[已隐藏]')
  assert.equal(data.nested[0].device_code, '[已隐藏]')
  assert.deepEqual(data.nested[0].roles, ['reader'])
  assert.equal(data.hasRefreshToken, true)
  assert.equal(data.tokenType, 'Bearer')
  assert.equal(input.access_token, 'private')
})

test('flow POST uses same-origin credentials and JSON, forbids redirects, and exposes only public response data', async t => {
  const originalFetch = globalThis.fetch
  t.after(() => { globalThis.fetch = originalFetch })
  let request
  globalThis.fetch = async (path, options) => {
    request = { path, ...options }
    return { ok: true, status: 200, json: async () => ({ granted: true, refresh_token: 'private' }) }
  }
  const reply = await apiRequest('/api/introspect', { flow: 'device' })
  assert.equal(request.path, '/api/introspect')
  assert.equal(request.method, 'POST')
  assert.equal(request.credentials, 'same-origin')
  assert.equal(request.redirect, 'error')
  assert.equal(request.cache, 'no-store')
  assert.equal(request.headers['Content-Type'], 'application/json')
  assert.deepEqual(JSON.parse(request.body), { flow: 'device' })
  assert.equal(reply.data.refresh_token, '[已隐藏]')
})

test('public config preserves the enabled legacy password capability while discarding all unlisted private fields', async t => {
  const originalFetch = globalThis.fetch
  t.after(() => { globalThis.fetch = originalFetch })
  const config = {
    publicOrigin: 'http://localhost:3010', platformOrigin: 'http://localhost:3000', scope: 'profile',
    client_secret: 'private', password: 'private',
    flows: {
      authorization_code: { enabled: true, code_verifier: 'private' },
      device: { enabled: true, device_code: 'private' },
      client_credentials: { enabled: 'true', clientSecret: 'private' },
      password: { enabled: true, password: 'private', clientSecret: 'private' },
      extra: { enabled: true },
    },
  }
  globalThis.fetch = async () => ({ ok: true, status: 200, json: async () => config })
  const reply = await apiRequest('/api/config')
  assert.equal(reply.data.flows.password.enabled, true)
  assert.deepEqual(reply.data, {
    publicOrigin: config.publicOrigin, platformOrigin: config.platformOrigin, scope: 'profile',
    flows: {
      authorization_code: { enabled: true }, device: { enabled: true },
      client_credentials: { enabled: false }, password: { enabled: true },
    },
  })
})

test('public config preserves the known disabled reason and rejects malformed capability values', async t => {
  const originalFetch = globalThis.fetch
  t.after(() => { globalThis.fetch = originalFetch })
  globalThis.fetch = async () => ({ ok: true, status: 200, json: async () => ({
    flows: { authorization_code: null, device: true, client_credentials: { enabled: false, reason: 'private' }, password: { enabled: false, reason: 'legacy_password_disabled' } },
  }) })
  const reply = await apiRequest('/api/config')
  assert.deepEqual(reply.data.flows, {
    authorization_code: { enabled: false }, device: { enabled: false },
    client_credentials: { enabled: false }, password: { enabled: false, reason: 'legacy_password_disabled' },
  })
})
