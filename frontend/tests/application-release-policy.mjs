import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'

const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const require = createRequire(path.join(repository, 'package.json'))
const ts = require('typescript')
function load(relative, dependencies = {}) {
  const source = readFileSync(path.join(repository, relative), 'utf8')
  const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText
  const exports = {}
  new vm.Script(`(function(require,exports){${compiled}\n})`, { filename: relative }).runInNewContext({ URL, Set })(
    name => dependencies[name] ?? require(name), exports)
  return exports
}
const policy = load('src/pages/system/applications/external-release-policy.ts')
const valid = (changes = {}) => ({ version: '1.0.0', entryUrl: '/sub-apps/stores/versions/1.0.0/index.html', healthUrl: '', resources: [], ...changes })
const json = value => JSON.parse(JSON.stringify(value))

test('URL registration produces the six existing fields and fixed HTML without deployment metadata', () => {
  const payload = policy.externalReleasePayload('production', valid({ version: ' 1.0.0 ', image: 'ignored', digest: 'ignored', target: 'ignored' }))
  assert.deepEqual(json(payload), { environment: 'production', version: '1.0.0', entryType: 'HTML',
    entryUrl: '/sub-apps/stores/versions/1.0.0/index.html', healthUrl: null, resources: [] })
})

test('version roots use a literal complete directory segment with no query aliases', () => {
  for (const entryUrl of ['/versions/1.0.01/index.html', '/versions/1.0.0', '/versions/current/index.html?v=1.0.0', '/versions/1.0.0/index.html#view']) {
    assert.equal(policy.externalReleaseSchema.safeParse(valid({ entryUrl })).success, false, entryUrl)
  }
  assert.equal(policy.externalReleaseSchema.safeParse(valid({ entryUrl: 'https://approved.example/app/1.0.0/' })).success, true)
})

test('entry, health and resource URLs reject bare query and fragment delimiters', () => {
  for (const suffix of ['?', '#']) {
    for (const entryUrl of [valid().entryUrl + suffix, 'https://approved.example/app/1.0.0/index.html' + suffix]) {
      assert.equal(policy.externalReleaseSchema.safeParse(valid({ entryUrl })).success, false, entryUrl)
    }
    for (const healthUrl of ['/healthz' + suffix, 'https://approved.example/healthz' + suffix]) {
      assert.equal(policy.externalReleaseSchema.safeParse(valid({ healthUrl })).success, false, healthUrl)
    }
    for (const url of ['assets/chunk.js' + suffix, '/app/1.0.0/chunk.js' + suffix, 'https://approved.example/app/1.0.0/chunk.js' + suffix]) {
      assert.equal(policy.externalReleaseSchema.safeParse(valid({ resources: [{ kind: 'SCRIPT', url }] })).success, false, url)
    }
  }
})

test('entry rejects protocol-relative, credentials, insecure absolute URLs, controls and traversal', () => {
  for (const entryUrl of ['//unapproved.example/1.0.0/', 'http://localhost/1.0.0/', 'https://user:secret@example.com/1.0.0/',
    'https://example.com/1.0.0/../index.html', '/1.0.0/%2fentry.html', '/1.0.0/\\entry.html', '/1.0.0/in\u0000dex.html', '/1.0.0/in dex.html']) {
    assert.equal(policy.externalReleaseSchema.safeParse(valid({ entryUrl })).success, false, JSON.stringify(entryUrl))
  }
})

test('health URL is empty, a gateway root path or an explicit HTTPS URL', () => {
  for (const healthUrl of ['', '/healthz', 'https://approved.example/healthz']) assert.equal(policy.externalReleaseSchema.safeParse(valid({ healthUrl })).success, true)
  for (const healthUrl of ['healthz', '//example.com/healthz', 'http://example.com/healthz', '/healthz?debug=1']) assert.equal(policy.externalReleaseSchema.safeParse(valid({ healthUrl })).success, false)
})

test('resource rows preserve kinds and relative paths and reject duplicate resolved URLs', () => {
  const resources = [{ kind: 'SCRIPT', url: 'assets/chunk.js' }, { kind: 'STYLE', url: 'assets/app.css' }, { kind: 'ASSET', url: 'image.png' }]
  assert.deepEqual(json(policy.externalReleasePayload('staging', valid({ resources })).resources), resources)
  for (const extra of [{ kind: 'EXEC', url: 'x' }, { kind: 'SCRIPT', url: '../outside.js' }, { kind: 'SCRIPT', url: 'javascript:alert(1)' }]) {
    assert.equal(policy.externalReleaseSchema.safeParse(valid({ resources: [extra] })).success, false)
  }
  assert.equal(policy.externalReleaseSchema.safeParse(valid({ resources: [resources[0], { kind: 'ASSET', url: '/sub-apps/stores/versions/1.0.0/assets/chunk.js' }] })).success, false)
})

test('explicit resources are bounded to 48 before any API request', () => {
  const resources = Array.from({ length: 48 }, (_, i) => ({ kind: 'SCRIPT', url: `assets/chunk-${i}.js` }))
  assert.equal(policy.externalReleaseSchema.safeParse(valid({ resources })).success, true)
  assert.equal(policy.externalReleaseSchema.safeParse(valid({ resources: [...resources, { kind: 'SCRIPT', url: 'overflow.js' }] })).success, false)
})

test('register API sends only the six supported fields and strips nested unknown resource fields', async () => {
  const calls = []
  const { applicationsApi } = load('src/api/applications.ts', { '@/lib/api': { api: { post: (...args) => { calls.push(args); return Promise.resolve({ validationStatus: 'DRAFT' }) } } } })
  const payload = { environment: 'production', version: '1.0.0', entryType: 'ESM', entryUrl: '/stores/1.0.0/index.html', healthUrl: null,
    resources: [{ kind: 'SCRIPT', url: 'chunk.js', digest: 'ignored' }], image: 'ignored', digest: 'ignored', target: 'ignored' }
  assert.equal((await applicationsApi.registerRelease('stores-id', payload)).validationStatus, 'DRAFT')
  assert.deepEqual(json(calls), [['/api/applications/stores-id/releases', { environment: 'production', version: '1.0.0', entryType: 'HTML',
    entryUrl: '/stores/1.0.0/index.html', healthUrl: null, resources: [{ kind: 'SCRIPT', url: 'chunk.js' }] }]])
})
