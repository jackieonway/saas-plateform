import assert from 'node:assert/strict'
import test from 'node:test'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const require = createRequire(path.join(root, 'package.json'))
const ts = require('typescript')
const source = fs.readFileSync(path.join(root, 'src/pages/system/applications/application-setup-policy.ts'), 'utf8')
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText
const policy = {}
vm.runInThisContext(`(function(exports){${compiled}\n})`)(policy)
const app = { id: 'app-a', name: 'CRM', code: 'crm', entryType: 'HTML', entryUrl: '/sub-apps/crm/unreleased/index.html' }
const status = changes => ({ applicationId: 'app-a', environment: 'development', applicationEnabled: true,
  entryType: 'HTML', runtimeMode: 'TRUSTED', releaseRequired: true, activeVersion: null, releaseValidated: false,
  menuCount: 2, entryMenuCount: 1, canConfigureAccess: true, access: null, ...changes })
const target = changes => ({ tenantId: 'tenant-a', organizationId: 'org-a', roleId: 'role-a', tenantAuthorized: true,
  organizationAuthorized: true, roleAuthorized: true, assignedUserCount: 1, ...changes })
const checks = changes => policy.applicationSetupChecks(app, status(changes))
const check = (items, id) => items.find(item => item.id === id)

test('setup links normalize unknown steps and environments to safe defaults', () => {
  assert.equal(policy.setupEnvironment, undefined)
  assert.equal(policy.setupStep('release'), 'release')
  assert.equal(policy.setupStep('access'), 'access')
  for (const value of [null, 'complete', '/system/users']) assert.equal(policy.setupStep(value), 'overview')
})

test('created metadata and page counts never imply a published release or granted audience', () => {
  const result = checks({})
  assert.equal(check(result, 'application').complete, true)
  assert.equal(check(result, 'menus').complete, true)
  assert.equal(check(result, 'release').complete, false)
  assert.equal(check(result, 'access').complete, false)
  assert.match(check(result, 'access').description, /选择租户/)
})

test('active version must also pass checks and executable menus must exist', () => {
  assert.equal(check(checks({ activeVersion: '1.0.0', releaseValidated: false }), 'release').complete, false)
  assert.equal(check(checks({ releaseValidated: true }), 'release').complete, false)
  assert.equal(check(checks({ activeVersion: '1.0.0', releaseValidated: true }), 'release').complete, true)
  assert.equal(check(checks({ menuCount: 8, entryMenuCount: 0 }), 'menus').complete, false)
})

test('one selected audience must have the complete same-entry grant chain and effective members', () => {
  for (const key of ['tenantAuthorized', 'organizationAuthorized', 'roleAuthorized']) {
    assert.equal(check(checks({ access: target({ [key]: false }) }), 'access').complete, false)
  }
  const empty = check(checks({ access: target({ assignedUserCount: 0 }) }), 'access')
  assert.equal(empty.complete, false)
  assert.match(empty.description, /尚无/)
  assert.equal(check(checks({ access: target({}) }), 'access').complete, true)
})

test('legacy incompatible modes never become ready even with an active version or direct address', () => {
  for (const legacy of [{ entryType: 'ESM', runtimeMode: 'TRUSTED' }, { entryType: 'HTML', runtimeMode: 'IFRAME' }]) {
    const result = checks({ ...legacy, releaseRequired: false, releaseValidated: true, activeVersion: '1.0.0', entrySource: 'URL' })
    assert.equal(check(result, 'release').complete, false)
    assert.match(check(result, 'release').description, /重新配置 qiankun/)
  }
})

test('a validated direct URL is ready without an active version and still requires audience grants', () => {
  const live = checks({ entrySource: 'URL', urlEntryConfigured: true, releaseRequired: false, releaseValidated: true,
    runtimeMode: 'TRUSTED', proxyEnabled: true })
  assert.equal(check(live, 'release').complete, true)
  assert.match(check(live, 'release').description, /直接地址.*代理/)
  assert.equal(check(live, 'access').complete, false)
  assert.equal(check(checks({ entrySource: 'URL', releaseRequired: false, releaseValidated: false }), 'release').complete, false)
})

test('a disabled application remains a visible incomplete prerequisite', () => {
  const result = checks({ applicationEnabled: false, activeVersion: '1.0.0', releaseValidated: true, access: target({}) })
  assert.equal(result.every(item => item.complete), false)
  assert.equal(check(result, 'application').complete, false)
  assert.match(check(result, 'application').description, /停用/)
})
