import assert from 'node:assert/strict'
import test from 'node:test'
import fs from 'node:fs'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import { componentFixture, elements, button } from './helpers/component-fixture.mjs'

const require = createRequire(import.meta.url)
const ts = require('typescript')
const load = (relative, dependencies = {}) => {
  const source = fs.readFileSync(new URL(`../${relative}`, import.meta.url), 'utf8')
  const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText
  const exports = {}
  vm.runInThisContext(`(function(require,exports){${compiled}\n})`)(name => dependencies[name] ?? require(name), exports)
  return exports
}
const { urlEntrySchema } = load('src/pages/system/applications/url-entry-policy.ts')
const setupPolicy = load('src/pages/system/applications/application-setup-policy.ts')
const app = { id: 'app-a', code: 'crm', name: 'CRM', basePath: '/micro/crm', status: 'ENABLED', entryType: 'HTML', runtimeMode: 'TRUSTED' }
const props = overrides => ({ application: app, environment: 'staging', entry: null, ready: true, busy: false,
  onBusyChange() {}, onChanged() {}, ...overrides })

function fixture(apiOverrides = {}) {
  const calls = [], notices = [], changes = []
  let form
  const f = componentFixture('src/pages/system/applications/ApplicationUrlEntryForm.tsx', ['ApplicationUrlEntryForm'], {
    './url-entry-policy': { urlEntrySchema },
    './application-setup-policy': setupPolicy,
    '@/lib/form': { zResolver: () => undefined },
    'react-hook-form': { useForm: options => form ??= {
      values: { ...options.defaultValues }, control: {},
      reset(values) { this.values = values },
      handleSubmit(callback) { return () => callback(this.values) },
    } },
    '@/api/applications': { applicationsApi: {
      setUrlEntry: async (...args) => { calls.push(['save', ...args]); if (apiOverrides.setUrlEntry) return apiOverrides.setUrlEntry(...args); return { applicationId: args[0], environment: args[1], ...args[2] } },
      removeUrlEntry: async (...args) => { calls.push(['remove', ...args]) },
      update: async (...args) => { calls.push(['update', ...args]) },
    } },
    sonner: { toast: { success: (...args) => notices.push(args), error: (...args) => notices.push(args) } },
  })
  return { ...f, calls, notices, changes, form: () => form,
    renderForm: overrides => f.render('ApplicationUrlEntryForm', props({ onChanged: () => changes.push(true), ...overrides })) }
}

test('live entries accept HTTP and HTTPS root or subdirectory addresses without a version directory', () => {
  for (const entryUrl of ['https://app1.example.com', 'https://app1.example.com/', 'http://localhost:5173/', 'https://apps.example/portal/', 'https://apps.example/portal/index.html?theme=dark']) {
    assert.equal(urlEntrySchema.safeParse({ entryUrl, proxyEnabled: true }).success, true)
  }
  for (const entryUrl of ['//example.com/', 'javascript:alert(1)', 'https://user:pass@example.com/',
    'https://example.com/?token=secret', 'https://example.com/#/route', 'https://example.com/with space',
    'https://example.com/portal', 'https://example.com/app.js', 'https://example.com/?__saas_execution=reserved',
    'https://example.com/?__saas_lease=reserved']) {
    assert.equal(urlEntrySchema.safeParse({ entryUrl, proxyEnabled: true }).success, false, entryUrl)
  }
})

test('live entry APIs keep environment selection in the URL and send only supported write fields', async () => {
  const calls = []
  const api = Object.fromEntries(['get', 'put', 'del'].map(method => [method, async (...args) => { calls.push([method, ...args]); return null }]))
  const { applicationsApi } = load('src/api/applications.ts', { '@/lib/api': { api,
    qs: values => `?${new URLSearchParams(values)}` } })
  assert.equal(await applicationsApi.urlEntry('id/with slash', 'development'), null)
  await applicationsApi.setUrlEntry('app-a', 'production', { entryUrl: 'http://upstream/', proxyEnabled: true, ignored: 'value' })
  await applicationsApi.removeUrlEntry('app-a', 'staging')
  assert.deepEqual(calls, [
    ['get', '/api/applications/id%2Fwith%20slash/url-entry?environment=development'],
    ['put', '/api/applications/app-a/url-entry?environment=production', { entryUrl: 'http://upstream/', proxyEnabled: true }],
    ['del', '/api/applications/app-a/url-entry?environment=staging'],
  ])
})

test('direct URL form defaults to the proxy and saves the selected environment', async () => {
  const f = fixture()
  const tree = f.renderForm()
  assert.equal(f.form().values.proxyEnabled, true)
  f.form().values.entryUrl = 'http://localhost:5173/'
  elements(tree, item => item.type === 'form')[0].props.onSubmit()
  await f.mutations[0].promise
  assert.deepEqual(f.calls, [['save', 'app-a', 'staging', { entryUrl: 'http://localhost:5173/', proxyEnabled: true }]])
  assert.equal(f.notices.length, 1)
})

test('unconfirmed entry reads cannot save and restoring a version uses the same selected environment', async () => {
  const f = fixture()
  let tree = f.renderForm({ ready: false })
  assert.equal(button(tree, '保存并使用').props.disabled, true)
  elements(tree, item => item.type === 'form')[0].props.onSubmit()
  assert.equal(f.calls.length, 0)
  tree = f.renderForm({ entry: { entryUrl: 'https://crm.example/', proxyEnabled: true, browserEntryUrl: '/micro-proxy/crm/' } })
  button(tree, '恢复版本入口').props.onClick()
  await f.mutations[1].promise
  assert.deepEqual(f.calls, [['remove', 'app-a', 'staging']])
})

test('a late save completion after unmount cannot notify or advance another editor', async () => {
  const f = fixture()
  f.renderForm()
  f.unmount()
  await f.mutations[0].options.onSuccess({ applicationId: app.id, environment: 'staging', entryUrl: 'https://crm.example/' })
  assert.equal(f.notices.length, 0)
  assert.equal(f.changes.length, 0)
})

test('explicit legacy reconfiguration saves qiankun metadata while disabled before writing the new entry', async () => {
  const f = fixture()
  const tree = f.renderForm({ application: { ...app, entryType: 'HTML', runtimeMode: 'IFRAME', status: 'DISABLED' } })
  assert.match(JSON.stringify(tree), /代码将在平台页面中执行/)
  const next = await f.mutations[0].options.mutationFn({ entryUrl: 'https://new.example/', proxyEnabled: true })
  assert.equal(f.calls[0][0], 'update')
  assert.equal(f.calls[0][2].entryType, 'HTML')
  assert.equal(f.calls[0][2].runtimeMode, 'TRUSTED')
  assert.equal(f.calls[0][2].status, 'DISABLED')
  assert.equal(f.calls[1][0], 'save')
  await f.mutations[0].options.onSuccess(next)
  assert.match(JSON.stringify(f.notices), /应用保持停用/)
  assert.equal(f.calls.some(call => call[2]?.status === 'ENABLED'), false)
})

test('a failed second step of reconfiguration is reported incomplete and never produces a success notice', async () => {
  const f = fixture({ setUrlEntry: async () => { throw new Error('upstream unavailable') } })
  f.renderForm({ application: { ...app, entryType: 'ESM', status: 'DISABLED' } })
  await assert.rejects(f.mutations[0].options.mutationFn({ entryUrl: 'https://new.example/', proxyEnabled: true }), /入口配置尚未完成/)
  assert.equal(f.calls[0][2].status, 'DISABLED')
  assert.equal(f.calls[1][0], 'save')
  assert.equal(f.notices.length, 0)
  assert.equal(f.changes.length, 0)
})
