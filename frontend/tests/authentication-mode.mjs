import assert from 'node:assert/strict'
import test from 'node:test'
import fs from 'node:fs'
import vm from 'node:vm'
import { createRequire } from 'node:module'

const require = createRequire(new URL('../package.json', import.meta.url))
const ts = require('typescript')
const source = fs.readFileSync(new URL('../src/lib/authentication-mode.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText

function fixture(handler) {
  const calls = [], redirects = [], exports = {}
  const sandbox = vm.createContext({ exports, AbortController, setTimeout, clearTimeout,
    fetch: async (path, options) => { calls.push({ path, options }); return handler(path, options) },
    window: { location: { assign: destination => redirects.push(destination) } },
  })
  new vm.Script(compiled).runInContext(sandbox)
  return { ...exports, calls, redirects }
}
const reply = (status, value) => new Response(JSON.stringify(value), { status, headers: { 'content-type': 'application/json' } })

for (const mode of ['BFF']) test(`authentication mode ${mode} requires an explicit server contract`, async () => {
  const f = fixture(async () => reply(200, { mode }))
  assert.equal(await f.loadAuthenticationMode(), mode)
  assert.equal(f.calls.length, 1)
  assert.equal(f.calls[0].path, '/auth/config')
  assert.equal(f.calls[0].options.credentials, 'same-origin')
  assert.equal(f.calls[0].options.cache, 'no-store')
  assert.equal(f.calls[0].options.redirect, 'error')
})

for (const [label, response] of [
  ['unavailable service', () => reply(503, { message: 'unavailable' })],
  ['anonymous response', () => reply(401, {})],
  ['retired password mode', () => reply(200, { mode: 'LEGACY' })],
  ['unrecognized contract', () => reply(200, { mode: 'AUTOMATIC_FALLBACK' })],
  ['missing mode', () => reply(200, {})],
  ['unreadable response', () => new Response('not-json', { status: 200 })],
  ['transport loss', () => { throw new TypeError('network unavailable') }],
]) test(`${label} cannot select a legacy password or refresh fallback`, async () => {
  const f = fixture(async () => response())
  await assert.rejects(f.loadAuthenticationMode(), /不会切换到其他登录方式/)
  assert.deepEqual(f.calls.map(call => call.path), ['/auth/config'])
  assert.deepEqual(f.redirects, [])
})

test('only known same-origin console navigation survives authentication', () => {
  const f = fixture(async () => { throw new Error('navigation helper must not fetch credentials') })
  for (const path of ['/', '/dashboard', '/system/users?status=ENABLED', '/micro/work-orders/tickets', '/device?user_code=owned']) {
    assert.equal(f.safePlatformReturn(path), path)
  }
  for (const path of [null, '', 'https://unregistered.invalid', '//unregistered.invalid', '/\\unregistered.invalid',
    '/%2funregistered.invalid', '/system/../auth/callback', '/system/%2e%2e/auth/callback', '/system/%252e%252e',
    '/auth/callback?code=not-a-browser-credential', '/api/auth/login', '/login', '/dashboard#untrusted', '/dashboard\n']) {
    assert.equal(f.safePlatformReturn(path), '/dashboard')
  }
})

test('explicit unified login navigates to the local BFF entry with a bounded return path', () => {
  const f = fixture(async () => { throw new Error('login navigation must not fetch tokens') })
  assert.deepEqual(f.redirects, [])
  f.startUnifiedLogin('/system/users?status=ENABLED')
  f.startUnifiedLogin('https://unregistered.invalid/callback')
  assert.deepEqual(f.redirects, ['/auth/login?return=%2Fsystem%2Fusers%3Fstatus%3DENABLED', '/auth/login?return=%2Fdashboard'])
  assert.deepEqual(f.calls, [])
})
