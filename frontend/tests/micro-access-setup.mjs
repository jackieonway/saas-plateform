import assert from 'node:assert/strict'
import test from 'node:test'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
import { componentFixture, success, failure, pending, elements, button } from './helpers/component-fixture.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const require = createRequire(path.join(root, 'package.json'))
const ts = require('typescript')
const relative = 'src/pages/system/applications/ApplicationAccessSetup.tsx'
const application = { id: 'app-a', code: 'orders', name: 'Orders', status: 'ENABLED' }
const tenant = { id: 'tenant-a', name: 'Tenant A', status: 'ENABLED' }
const organization = { id: 'org-a', tenantId: tenant.id, name: 'Org A', status: 'ENABLED', children: [] }
const role = { id: 'role-a', tenantId: tenant.id, organizationId: organization.id, name: 'Operators', status: 'ENABLED', code: 'operators', builtIn: false }
const menus = [
  { id: 'menu-home', parentId: null, title: 'Home', type: 'MENU', path: '/micro/orders', authorized: false },
  { id: 'menu-write', parentId: null, title: 'Write', type: 'BUTTON', path: '', authorized: false },
]
const target = { tenantId: tenant.id, organizationId: organization.id, roleId: role.id }
const props = { application, environment: 'development', canConfigureAccess: true }
const setup = overrides => ({
  applicationId: application.id, environment: 'development', applicationEnabled: true, entryType: 'HTML', runtimeMode: 'TRUSTED',
  releaseRequired: true, activeVersion: null, releaseValidated: false, menuCount: 2, entryMenuCount: 1, canConfigureAccess: true,
  access: { ...target, tenantAuthorized: false, organizationAuthorized: false, roleAuthorized: false, assignedUserCount: 0 }, ...overrides,
})
const plainPage = data => Array.isArray(data) ? { records: data, total: data.length } : data
function flatten(nodes) { return nodes.flatMap(node => [node, ...flatten(node.children ?? [])]) }
function fixture(dependencies = {}) {
  const writes = [], reads = [], errors = [], configured = []
  const status = setup({ access: { ...target, tenantAuthorized: true, organizationAuthorized: true, roleAuthorized: true, assignedUserCount: 0 } })
  const f = componentFixture(relative, ['ApplicationAccessSetupContent', 'loadRoleCandidates', 'matchingStatus'], {
    '@/api/application-setup': { applicationSetupApi: {
      organizations: async () => [organization],
      grantAccess: async (...args) => { writes.push(args); return { ok: true } },
      status: async (...args) => { reads.push(args); return status },
    } },
    '@/lib/api': { asPage: plainPage, asArray: data => data },
    '@/components/EntityTreePicker': { EntityTreePicker: 'EntityTreePicker', flattenPicker: flatten },
    '@/components/TreeCheck': { TreeCheck: 'TreeCheck', fromAuthMenus: data => data.map(item => ({ ...item, children: [] })) },
    '@/lib/tree': { buildTree: data => data.map(node => ({ node, children: [] })) },
    sonner: { toast: { success() {}, error: message => errors.push(message) } },
    ...dependencies,
  })
  f.queries.set('application-access-tenants', success([tenant]))
  f.queries.set('application-access-organizations', success([organization]))
  f.queries.set('application-access-roles', success([role]))
  f.queries.set('application-access-menus', success(menus))
  f.queries.set('application-setup-target', success(setup()))
  const currentProps = { ...props, onConfigured: () => configured.push(true) }
  const render = () => f.render('ApplicationAccessSetupContent', currentProps)
  return { f, writes, reads, errors, configured, status, currentProps, render }
}
function chooseAll(context) {
  let tree = context.render()
  elements(tree, item => item.type === 'Select')[0].props.onValueChange(tenant.id)
  tree = context.render()
  elements(tree, item => item.type === 'EntityTreePicker')[0].props.onSelect(organization.id)
  tree = context.render()
  elements(tree, item => item.type === 'Select')[1].props.onValueChange(role.id)
  return context.render()
}
const request = menuIds => ({ applicationId: application.id, environment: 'development', payload: { ...target, menuIds } })

test('access configuration starts with explicit targets and only preselects the current application home', () => {
  const context = fixture()
  let tree = context.render()
  assert.equal(button(tree, '确认授予访问').props.disabled, true)
  tree = chooseAll(context)
  assert.equal(button(tree, '确认授予访问').props.disabled, false)
  const menuTree = elements(tree, item => item.type === 'TreeCheck')[0]
  assert.deepEqual([...menuTree.props.checked], ['menu-home'])
})

test('grant writes the exact confirmed target and selected menus, then reads real target status', async () => {
  const context = fixture()
  let tree = chooseAll(context)
  elements(tree, item => item.type === 'TreeCheck')[0].props.onToggle(['menu-write'], true)
  tree = context.render()
  button(tree, '确认授予访问').props.onClick()
  await context.f.mutations[0].promise
  assert.deepEqual(context.writes, [[application.id, { ...target, menuIds: ['menu-home', 'menu-write'] }]])
  assert.deepEqual(context.reads, [[application.id, 'development', target]])
  tree = context.render()
  assert.equal(context.configured.length, 1)
  assert.match(JSON.stringify(tree), /授权已保存，继续检查访问条件/)
  assert.match(JSON.stringify(tree), /尚无可用入口/)
  assert.match(JSON.stringify(tree), /前往角色管理分配成员/)
  assert.doesNotMatch(JSON.stringify(tree), /当前范围已具备访问条件/)
})

test('failed or partial reads cannot submit an empty replacement or grant a foreign menu', async () => {
  const context = fixture()
  let tree = chooseAll(context)
  context.f.queries.set('application-access-menus', failure())
  tree = context.render()
  assert.equal(button(tree, '确认授予访问').props.disabled, true)
  await assert.rejects(context.f.mutations[0].options.mutationFn(request(['menu-home'])), /完整选择/)
  context.f.queries.set('application-access-menus', success(menus))
  tree = context.render()
  elements(tree, item => item.type === 'TreeCheck')[0].props.onToggle(['menu-home'], false)
  tree = context.render()
  assert.equal(button(tree, '确认授予访问').props.disabled, true)
  await assert.rejects(context.f.mutations[0].options.mutationFn(request(['menu-foreign'])), /完整选择/)
  assert.deepEqual(context.writes, [])
})

test('tenant changes synchronously clear organization and role so stale candidate responses do not authorize', async () => {
  const context = fixture()
  let tree = chooseAll(context)
  const stale = context.f.mutations[0].options.mutationFn
  const secondTenant = { ...tenant, id: 'tenant-b', name: 'Tenant B' }
  context.f.queries.set('application-access-tenants', success([tenant, secondTenant]))
  tree = context.render()
  elements(tree, item => item.type === 'Select')[0].props.onValueChange(secondTenant.id)
  await assert.rejects(stale(request(['menu-home'])), /完整选择/)
  tree = context.render()
  assert.equal(elements(tree, item => item.type === 'EntityTreePicker')[0].props.value, '')
  assert.equal(elements(tree, item => item.type === 'Select')[1].props.value, '')
  assert.equal(button(tree, '确认授予访问').props.disabled, true)
  assert.deepEqual(context.writes, [])
})

test('a confirmed access target is restored when the wizard returns from another step', async () => {
  const context = fixture(), changes = []
  context.currentProps.onTargetChange = value => changes.push(value)
  const tree = chooseAll(context)
  assert.deepEqual(changes.at(-1), target)
  button(tree, '确认授予访问').props.onClick()
  await context.f.mutations[0].promise
  const retained = changes.at(-1)
  context.f.unmount()

  const returned = fixture(), restored = []
  returned.currentProps.initialTarget = retained
  returned.currentProps.onTargetChange = value => restored.push(value)
  const nextTree = returned.render()
  assert.equal(elements(nextTree, item => item.type === 'Select')[0].props.value, tenant.id)
  assert.equal(elements(nextTree, item => item.type === 'EntityTreePicker')[0].props.value, organization.id)
  assert.equal(elements(nextTree, item => item.type === 'Select')[1].props.value, role.id)
  assert.deepEqual(restored, [target])
  assert.deepEqual(returned.f.queryOptions.get('application-setup-target').queryKey.slice(-3), Object.values(target))
})

test('restored targets are retained while candidate reads load or fail and emitted only after validation', () => {
  const context = fixture(), changes = []
  context.currentProps.initialTarget = target
  context.currentProps.onTargetChange = value => changes.push(value)
  context.f.queries.set('application-access-organizations', pending())
  context.f.queries.set('application-access-roles', pending())
  let tree = context.render()
  assert.equal(elements(tree, item => item.type === 'EntityTreePicker')[0].props.value, organization.id)
  assert.deepEqual(changes, [])
  assert.equal(button(tree, '确认授予访问').props.disabled, true)
  context.f.queries.set('application-access-organizations', failure())
  context.render()
  assert.deepEqual(changes, [])
  context.f.queries.set('application-access-organizations', success([organization]))
  context.f.queries.set('application-access-roles', success([role]))
  context.render()
  assert.deepEqual(changes, [target])
})

test('changing an organization immediately retires the parent target before the next render', () => {
  const context = fixture(), changes = []
  context.currentProps.onTargetChange = value => changes.push(value)
  let tree = chooseAll(context)
  assert.deepEqual(changes.at(-1), target)
  elements(tree, item => item.type === 'EntityTreePicker')[0].props.onSelect('org-b')
  assert.equal(changes.at(-1), null)
  tree = context.render()
  assert.equal(elements(tree, item => item.type === 'Select')[1].props.value, '')
  assert.equal(changes.at(-1), null)
  assert.equal(button(tree, '确认授予访问').props.disabled, true)
})

test('managed administrator roles cannot be selected through the simple access form', () => {
  const context = fixture()
  context.f.queries.set('application-access-roles', success([{ ...role, code: 'tenant_admin', builtIn: true }]))
  const tree = chooseAll(context)
  assert.equal(button(tree, '确认授予访问').props.disabled, true)
})

test('roles load all pages and omit disabled, foreign and managed targets', async () => {
  const calls = []
  const records = Array.from({ length: 200 }, (_, index) => ({ ...role, id: `role-${index}` }))
  const context = fixture({ '@/api/roles': { rolesApi: { list: async parameters => {
    calls.push(parameters)
    return { records: parameters.page === 1 ? records : [role, { ...role, id: 'managed', builtIn: true, code: 'org_admin' }, { ...role, id: 'disabled', status: 'DISABLED' }, { ...role, id: 'foreign', organizationId: 'org-b' }], total: 204 }
  } } } })
  const roles = await context.f.exports.loadRoleCandidates(tenant.id, organization.id)
  assert.equal(calls.length, 2)
  assert.equal(calls[1].page, 2)
  assert.equal(roles.length, 201)
  assert.equal(roles.some(item => ['managed', 'disabled', 'foreign'].includes(item.id)), false)
})

test('incomplete role pagination fails rather than presenting a partial candidate set', async () => {
  const context = fixture({ '@/api/roles': { rolesApi: { list: async ({ page }) => ({ records: page === 1 ? [role] : [], total: 2 }) } } })
  await assert.rejects(context.f.exports.loadRoleCandidates(tenant.id, organization.id), /未完整返回/)
})

test('menu candidates reject a response for another application', async () => {
  const context = fixture({ '@/api/tenants': { tenantsApi: { authorizations: async () => ({ applications: [{ id: 'foreign' }], menus: { foreign: menus } }) } } })
  chooseAll(context)
  await assert.rejects(context.f.queryOptions.get('application-access-menus').queryFn(), /菜单信息不完整/)
})

test('missing access permission disables candidate requests and displays administrator guidance', () => {
  const context = fixture()
  context.currentProps.canConfigureAccess = false
  const tree = context.render()
  assert.match(JSON.stringify(tree), /请由授权管理员/)
  assert.equal(context.f.queryOptions.get('application-access-tenants').enabled, false)
  assert.equal(context.f.queryOptions.get('application-access-organizations').enabled, false)
  assert.equal(context.f.queryOptions.get('application-access-roles').enabled, false)
  assert.equal(context.f.queryOptions.get('application-access-menus').enabled, false)
  assert.equal(button(tree, '确认授予访问'), undefined)
})

test('late successful grants after unmount cannot update a replacement wizard', async () => {
  let resolveGrant
  const pending = new Promise(resolve => { resolveGrant = resolve })
  const context = fixture({ '@/api/application-setup': { applicationSetupApi: { grantAccess: () => pending, status: async () => setup() } } })
  const tree = chooseAll(context)
  button(tree, '确认授予访问').props.onClick()
  await Promise.resolve()
  context.f.unmount()
  resolveGrant({ ok: true })
  await context.f.mutations[0].promise
  assert.deepEqual(context.configured, [])
})

test('target status rejects an application, environment or role mismatch', async () => {
  const context = fixture()
  assert.equal(context.f.exports.matchingStatus(setup(), application.id, 'development', target), true)
  for (const invalid of [setup({ applicationId: 'other' }), setup({ environment: 'production' }), setup({ access: { ...target, roleId: 'other' } })]) {
    assert.equal(context.f.exports.matchingStatus(invalid, application.id, 'development', target), false)
  }
})

test('a legacy incompatible entry cannot be reported as accessible', () => {
  const context = fixture()
  context.currentProps.application = { ...application, entryType: 'ESM' }
  context.currentProps.entryReady = false
  context.f.queries.set('application-setup-target', success(setup({
    entryType: 'ESM', runtimeMode: 'TRUSTED', releaseRequired: false, releaseValidated: true,
    access: { ...target, tenantAuthorized: true, organizationAuthorized: true, roleAuthorized: true, assignedUserCount: 1 },
  })))
  let tree = chooseAll(context)
  assert.doesNotMatch(JSON.stringify(tree), /当前范围已具备访问条件/)
  assert.match(JSON.stringify(tree), /重新配置 qiankun/)
  context.currentProps.entryReady = true
  tree = context.render()
  assert.doesNotMatch(JSON.stringify(tree), /当前范围已具备访问条件/)
})

test('application setup transport forwards only an explicit target and additive menu payload', async () => {
  const source = fs.readFileSync(path.join(root, 'src/api/application-setup.ts'), 'utf8')
  const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText
  const exports = {}, calls = []
  const qs = values => '?' + new URLSearchParams(values).toString()
  const api = { get: async url => { calls.push(['GET', url]); return {} }, post: async (url, body) => { calls.push(['POST', url, body]); return { ok: true } } }
  vm.runInThisContext(`(function(require,exports){${compiled}\n})`)(() => ({ api, qs }), exports)
  await exports.applicationSetupApi.status(application.id, 'development', { ...target, menuIds: ['menu-home'] })
  await exports.applicationSetupApi.grantAccess(application.id, { ...target, menuIds: ['menu-home'] })
  await exports.applicationSetupApi.organizations(application.id, tenant.id)
  assert.match(calls[0][1], /setup\?environment=development&tenantId=tenant-a&organizationId=org-a&roleId=role-a$/)
  assert.deepEqual(calls[1], ['POST', '/api/applications/app-a/setup/access', { ...target, menuIds: ['menu-home'] }])
  assert.deepEqual(calls[2], ['GET', '/api/applications/app-a/setup/organizations?tenantId=tenant-a'])
})
