import assert from 'node:assert/strict'
import test from 'node:test'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const require = createRequire(path.join(root, 'frontend/package.json'))
const ts = require('typescript'), modules = new Map()
function load(relative) {
  if (modules.has(relative)) return modules.get(relative)
  const source = fs.readFileSync(path.join(root, relative), 'utf8')
  const compiled = ts.transpileModule(source, { compilerOptions: {
    module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022,
  } }).outputText
  const exports = {}; modules.set(relative, exports)
  const execute = vm.runInThisContext(`(function(require,exports){${compiled}\n})`, { filename: relative })
  execute(name => name.startsWith('@/') ? load(`frontend/src/${name.slice(2)}.ts`) : require(name), exports)
  return exports
}
const { evaluateRouteAccess, collectAuthorizedPaths, routeMenuAccess, externalMenuUrl } = load('frontend/src/lib/route-policy.ts')
const { resolveRoutePolicy } = load('frontend/src/lib/route-permissions.ts')
const menu = (path, extra = {}) => ({ id: path, type: 'MENU', path, children: [], ...extra })
const success = data => ({ status: 'success', data })
function input(menus = [], extra = {}) {
  return { profile: { platformContext: true, tenant: { isPlatform: true }, applications: [
    { id: 'system', code: 'system' }, { id: 'orders', code: 'work-orders' }, { id: 'other', code: 'other' },
  ] }, permissions: success({ permissions: ['system:oauth', 'system:user'], roles: [] }), applicationMenus: [
    { applicationId: 'system', ...success(menus) },
  ], ...extra }
}
const decision = (pathname, value, search) => evaluateRouteAccess({ ...value, pathname, search })
const micro = (menus, extra = {}) => input([], { applicationMenus: [{ applicationId: 'orders', ...success(menus) }], ...extra })

test('data grants page requires its own read permission and exact MENU evidence without broad management rights', () => {
  const path = '/system/data-grants'
  const allowed = input([menu(path)], { permissions: success({ permissions: ['system:data-grant:read'], roles: [] }) })
  assert.equal(decision(path, allowed).status, 'allowed')
  assert.equal(decision(path, { ...allowed, profile: { ...allowed.profile, platformContext: false } }).status, 'allowed')
  for (const permissions of [['system:user', 'system:authorization', 'system:application'], ['system:data-grant:write'], []]) {
    assert.equal(decision(path, { ...allowed, permissions: success({ permissions, roles: [] }) }).status, 'denied')
  }
  assert.equal(decision(path, input([menu(path, { type: 'BUTTON' })], { permissions: allowed.permissions })).status, 'denied')
  assert.equal(decision(path, { ...allowed, permissions: { status: 'error' } }).status, 'error')
})

test('typed route registry explicitly distinguishes personal, known host, micro, embed, root redirect and unknown', () => {
  for (const [path, kind] of [['/dashboard', 'personal'], ['/system/users', 'host'], ['/micro/work-orders/tickets/42', 'micro'],
    ['/embed', 'embed'], ['/', 'redirect'], ['/system/unregistered', 'unknown']]) assert.equal(resolveRoutePolicy(path).kind, kind)
  assert.equal(decision('/', input()).to, '/dashboard')
})

test('the exact personal allowlist ignores unrelated permission/menu errors but waits during context switching', () => {
  const value = input([], { permissions: { status: 'error' }, applicationMenus: [{ applicationId: 'system', status: 'error' }] })
  for (const path of ['/dashboard', '/profile', '/system/notifications']) {
    assert.equal(decision(path, value).status, 'allowed')
    assert.equal(decision(path, { ...value, switching: true }).status, 'pending')
    assert.equal(decision(path, { ...value, profile: null }).status, 'pending')
  }
})

test('host permission and exact authorized MENU are independently required', () => {
  assert.equal(decision('/system/users', input([menu('/system/users')])).status, 'allowed')
  assert.equal(decision('/system/users', input()).status, 'denied')
  assert.equal(decision('/system/users', input([menu('/system/users')], { permissions: success({ permissions: [], roles: [] }) })).status, 'denied')
})

test('a shared host permission cannot authorize its unlisted sibling MENU', () => {
  const value = input([menu('/system/oauth/clients')])
  assert.equal(decision('/system/oauth/clients', value).status, 'allowed')
  assert.equal(decision('/system/oauth/tokens', value).status, 'denied')
})

test('explicit platformContext false overrides the physical platform-tenant fallback', () => {
  const value = input([menu('/system/oauth/clients')])
  value.profile.platformContext = false
  assert.equal(decision('/system/oauth/clients', value).status, 'denied')
  assert.equal(collectAuthorizedPaths(value).has('/system/oauth/clients'), false)
})

test('a positive known MENU wins unrelated pending/error results while missing authorization stays closed', () => {
  for (const status of ['pending', 'error']) {
    const value = input([menu('/system/users')])
    value.applicationMenus.push({ applicationId: 'other', status })
    assert.equal(decision('/system/users', value).status, 'allowed')
    assert.equal(decision('/system/oauth/clients', value).status, status)
  }
})

test('failed permission or menu revalidation cannot grant access using retained cached data', () => {
  const value = input([menu('/system/users')])
  value.permissions.status = 'error'
  assert.equal(decision('/system/users', value).status, 'error')
  value.permissions.status = 'success'; value.applicationMenus[0].status = 'error'
  assert.equal(decision('/system/users', value).status, 'error')
})

test('DIR, BUTTON, parent routes and unknown MENU paths do not become executable host destinations', () => {
  const value = input([menu('/system', { type: 'DIR' }), menu('/system/users', { type: 'BUTTON' }), menu('/unknown')])
  for (const path of ['/system', '/system/users', '/unknown']) assert.equal(decision(path, value).status, 'denied')
  assert.deepEqual([...collectAuthorizedPaths(value)].sort(), ['/dashboard', '/profile', '/system/notifications'])
})

test('case, encoded segments and trailing slash use the same known route identity', () => {
  const value = input([menu('/SYSTEM/USERS/')])
  assert.equal(decision('/system/%75sers/', value).status, 'allowed')
  assert.equal(decision('/system/users/unlisted', value).status, 'denied')
})

test('micro exact routes and explicit parameter/wildcard MENU patterns authorize only their matching application', () => {
  const value = micro([menu('/micro/work-orders/tickets/:id'), menu('/micro/work-orders/reports/*')])
  for (const path of ['/micro/work-orders/tickets/42', '/micro/work-orders/reports/2026/10']) assert.equal(decision(path, value).status, 'allowed')
  for (const path of ['/micro/other/tickets/42', '/micro/work-orders-other/tickets/42']) assert.notEqual(decision(path, value).status, 'allowed')
})

test('micro parent MENU and application grant never authorize an unlisted descendant', () => {
  const value = micro([menu('/micro/work-orders')])
  assert.equal(decision('/micro/work-orders', value).status, 'allowed')
  assert.equal(decision('/micro/work-orders/private', value).status, 'denied')
})

test('detail read MENU cannot match the separately named work-orders creation route', () => {
  const value = micro([menu('/micro/work-orders/tickets/:id')])
  assert.equal(decision('/micro/work-orders/tickets/42', value).status, 'allowed')
  assert.equal(decision('/micro/work-orders/ticket-create', value).status, 'denied')
  value.applicationMenus[0].data.push(menu('/micro/work-orders/ticket-create'))
  assert.equal(decision('/micro/work-orders/ticket-create', value).status, 'allowed')
})

test('micro base redirects only to a concrete explicit child MENU, never a parameter or a parent fallback', () => {
  const value = micro([menu('/micro/work-orders/tickets/:id'), menu('/micro/work-orders/tickets')])
  assert.deepEqual(decision('/micro/work-orders', value), { status: 'redirect', to: '/micro/work-orders/tickets' })
  assert.equal(decision('/micro/work-orders', micro([menu('/micro/work-orders/tickets/:id')])).status, 'denied')
})

test('micro query error cannot use cached routes and unrelated app menus cannot grant micro access', () => {
  const value = micro([menu('/micro/work-orders/tickets')])
  value.applicationMenus[0].status = 'error'
  assert.equal(decision('/micro/work-orders/tickets', value).status, 'error')
  value.applicationMenus = [{ applicationId: 'system', ...success([menu('/micro/work-orders/tickets')]) }]
  assert.equal(decision('/micro/work-orders/tickets', value).status, 'pending')
})

test('embed authorization compares canonical HTTP(S) URLs exactly and survives unrelated menu failure', () => {
  const value = input([menu('/external', { linkUrl: 'https://EXAMPLE.test:443/private' })])
  value.applicationMenus.push({ applicationId: 'other', status: 'error' })
  assert.equal(decision('/embed', value, '?url=https%3A%2F%2Fexample.test%2Fprivate').status, 'allowed')
  assert.equal(decision('/embed', input([menu('/external', { linkUrl: 'https://example.test/public' })]), '?url=https://example.test/private').status, 'denied')
  assert.equal(externalMenuUrl('javascript:alert(1)'), null)
  assert.equal(decision('/embed', value, '?url=javascript:alert(1)').status, 'denied')
})

test('external SELF and BLANK entries use the same explicit URL MENU decision as direct embed', () => {
  for (const linkTarget of ['SELF', 'BLANK']) {
    const node = menu('/external', { linkUrl: 'https://example.test/private', linkTarget })
    assert.equal(routeMenuAccess(node, input([node])), true)
    assert.equal(routeMenuAccess(node, input()), false)
    assert.equal(routeMenuAccess({ ...node, type: 'BUTTON' }, input([node])), false)
  }
})

test('menu evidence for an application absent from the active profile cannot grant host or embed access', () => {
  const value = input([], { applicationMenus: [{ applicationId: 'removed-app', ...success([
    menu('/system/users'), menu('/external', { linkUrl: 'https://example.test/private' }),
  ]) }] })
  assert.equal(decision('/system/users', value).status, 'denied')
  assert.equal(decision('/embed', value, '?url=https://example.test/private').status, 'denied')
})

test('navigation path collection and direct route decisions agree on all returned executable paths', () => {
  const value = input([menu('/system/users'), menu('/system/oauth/clients'), menu('/system/unregistered'),
    menu('/system/oauth/tokens', { type: 'BUTTON' })])
  for (const path of collectAuthorizedPaths(value)) assert.equal(decision(path, value).status, 'allowed')
  assert.equal(collectAuthorizedPaths(value).has('/system/unregistered'), false)
  assert.equal(collectAuthorizedPaths(value).has('/system/oauth/tokens'), false)
})
