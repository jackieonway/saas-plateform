import assert from 'node:assert/strict'
import test from 'node:test'
import fs from 'node:fs'
import vm from 'node:vm'
import path from 'node:path'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const require = createRequire(path.join(root, 'frontend/package.json'))
const ts = require('typescript')
const deferred = () => { let resolve; const promise = new Promise(r => { resolve = r }); return { promise, resolve } }
const json = (status, body) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } })
function fixture(handler) {
  const storage = new Map(), redirects = [], calls = []
  const sandbox = vm.createContext({ Response, URLSearchParams, DOMException, AbortController, Promise,
    localStorage: { getItem: key => storage.get(key) ?? null, setItem: (key, value) => storage.set(key, value), removeItem: key => storage.delete(key) },
    window: { location: { pathname: '/system/users', search: '', assign: value => redirects.push(value) } },
    fetch: async (url, options) => { calls.push({ url, options }); return handler(url, options) },
  })
  function load(relative, dependencies) {
    const source = fs.readFileSync(path.join(root, relative), 'utf8')
    const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText
    const exports = {}
    const wrapper = new vm.Script(`(function(require,exports){${compiled}\n})`, { filename: relative }).runInContext(sandbox)
    wrapper(name => dependencies[name] ?? require(name), exports)
    return exports
  }
  const { authStore } = load('frontend/src/stores/auth.ts', {})
  const api = load('frontend/src/lib/api.ts', { '@/stores/auth': { authStore } })
  authStore.getState().setSession('expired-access', 'psr_test-refresh')
  return { authStore, api, storage, redirects, calls }
}

test('concurrent expired requests share one refresh, retry with rotated token and preserve context', async () => {
  const ready = deferred(), release = deferred(); let refreshes = 0
  const f = fixture(async (url, options) => {
    if (url === '/oauth/token') { refreshes++; ready.resolve(); await release.promise; assert.equal(new URLSearchParams(options.body).get('grant_type'), 'refresh_token'); return json(200, { access_token: 'renewed-access', refresh_token: 'psr_rotated', expires_in: 3600 }) }
    return options.headers.Authorization === 'Bearer renewed-access' ? json(200, { ok: true }) : json(401, { message: 'expired' })
  })
  f.authStore.getState().setCurrentApplication('authorized-app')
  const epoch = f.authStore.getState().contextEpoch
  const first = f.api.api.get('/api/users'), second = f.api.api.get('/api/roles')
  await ready.promise; release.resolve();
  assert.equal((await first).ok, true); assert.equal((await second).ok, true)
  assert.equal(refreshes, 1); assert.equal(f.authStore.getState().contextEpoch, epoch)
  assert.equal(f.authStore.getState().currentApplicationId, 'authorized-app')
  assert.equal(f.storage.get('dante-refresh-token'), 'psr_rotated'); assert.equal(f.redirects.length, 0)
})

test('retry occurs only once and a still-invalid session clears all credentials', async () => {
  let refreshes = 0
  const f = fixture(async url => url === '/oauth/token' ? (refreshes++, json(200, { access_token: 'new', refresh_token: 'psr_new' })) : json(401, { message: 'revoked' }))
  await assert.rejects(f.api.api.get('/api/users'), error => error.status === 401)
  assert.equal(refreshes, 1); assert.equal(f.authStore.getState().token, null)
  assert.equal(f.storage.has('dante-refresh-token'), false); assert.equal(f.redirects.length, 1)
})

test('late refresh from previous tenant cannot overwrite or log out a new session', async () => {
  const started = deferred(), release = deferred()
  const f = fixture(async url => { if (url === '/oauth/token') { started.resolve(); await release.promise; return json(200, { access_token: 'old-rotated', refresh_token: 'psr_old' }) } return json(401, {}) })
  const pending = f.api.api.get('/api/users'); await started.promise
  f.authStore.getState().setSession('tenant-B-access', 'psr_tenant-B')
  release.resolve(); await assert.rejects(pending, error => error.name === 'AbortError')
  assert.equal(f.authStore.getState().token, 'tenant-B-access'); assert.equal(f.redirects.length, 0)
})

test('login credential tickets never trigger platform-session refresh', async () => {
  const f = fixture(async () => json(401, { message: 'ticket expired' }))
  await assert.rejects(f.api.api.post('/api/auth/context', {}, { bearer: 'credential-ticket', suppress401: true }), error => error.status === 401)
  assert.equal(f.calls.length, 1); assert.equal(f.authStore.getState().token, 'expired-access'); assert.equal(f.redirects.length, 0)
})

test('late successful data response after context switch is discarded', async () => {
  const started = deferred(), release = deferred()
  const f = fixture(async () => { started.resolve(); await release.promise; return json(200, { tenantData: 'old' }) })
  const pending = f.api.api.get('/api/tenants'); await started.promise
  f.authStore.getState().beginContextSwitch(); release.resolve()
  await assert.rejects(pending, error => error.name === 'AbortError')
})

for (const failure of ['network', 'server']) test(`temporary refresh ${failure} failure preserves login for retry`, async () => {
  const f = fixture(async url => { if (url === '/oauth/token') { if (failure === 'network') throw new Error('offline'); return json(503, {}) } return json(401, {}) })
  await assert.rejects(f.api.api.get('/api/users'), error => error.code === (failure === 'network' ? 'NETWORK_ERROR' : 'SESSION_REFRESH_FAILED'))
  assert.equal(f.authStore.getState().token, 'expired-access'); assert.equal(f.redirects.length, 0)
})

test('expired access can refresh while switching context without ending the switch', async () => {
  const f = fixture(async url => url === '/oauth/token' ? json(200, { access_token: 'renewed', refresh_token: 'psr_new' }) :
    f.authStore.getState().token === 'renewed' ? json(200, { accessToken: 'selected-org', refreshToken: 'psr_selected' }) : json(401, {}))
  f.authStore.getState().beginContextSwitch()
  const result = await f.api.api.post('/api/auth/context', { organizationId: 'other-org' }, { bearer: 'expired-access', suppress401: true })
  assert.equal(result.accessToken, 'selected-org'); assert.equal(f.authStore.getState().contextSwitching, true)
  f.authStore.getState().setSession(result.accessToken, result.refreshToken)
  assert.equal(f.authStore.getState().contextSwitching, false)
})

test('invalid refresh clears credentials and legacy setToken removes old refresh/app state', async () => {
  const f = fixture(async url => json(url === '/oauth/token' ? 400 : 401, {}))
  await assert.rejects(f.api.api.get('/api/users'), error => error.code === 'SESSION_REFRESH_FAILED')
  assert.equal(f.authStore.getState().token, null); assert.equal(f.authStore.getState().refreshToken, null)
  f.authStore.getState().setSession('first', 'psr_first'); f.authStore.getState().setCurrentApplication('app')
  f.authStore.getState().setToken('legacy')
  assert.equal(f.authStore.getState().refreshToken, null); assert.equal(f.authStore.getState().currentApplicationId, null)
})
