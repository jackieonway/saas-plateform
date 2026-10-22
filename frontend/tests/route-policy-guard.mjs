import assert from 'node:assert/strict'
import test from 'node:test'
import fs from 'node:fs'
import vm from 'node:vm'
import path from 'node:path'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const require = createRequire(path.join(root, 'package.json'))
const ts = require('typescript')
const sourceRoot = process.env.ROUTE_POLICY_SOURCE_ROOT || root
const menu = (path, extra = {}) => ({ id: path, type: 'MENU', path, children: [], ...extra })
function guard({ pathname, search = '', menus = [], permissions = ['system:oauth'], platformContext = true }) {
  const profile = { applications: [{ id: 'host', code: 'system' }, { id: 'other', code: 'other' }],
    tenant: { isPlatform: true }, platformContext }
  const state = { profile, token: 'token', contextEpoch: 1, contextSwitching: false }
  const queries = menus.map((item, index) => ({ applicationId: index ? 'other' : 'host',
    isSuccess: item.status === 'success', isPending: item.status === 'pending', isError: item.status === 'error', ...item }))
  const permissionQuery = { status: 'success', isSuccess: true, data: { permissions }, refetch() {} }
  const jsx = (type, props) => ({ type, props })
  const deps = {
    'react/jsx-runtime': { jsx, jsxs: jsx },
    'react-router-dom': { ...require('react-router-dom'), useLocation: () => ({ pathname, search }), Navigate: 'redirect' },
    'lucide-react': { Loader2: 'loading-icon', ShieldAlert: 'denied-icon' },
    '@tanstack/react-query': { useQueries: () => queries },
    '@/stores/auth': { useAuthStore: selector => selector(state) },
    '@/hooks/use-auth-data': { usePermissionsQuery: () => permissionQuery, useMenusQuery: () => queries[0],
      useApplicationMenusQueries: () => ({ profile, switching: false, queries }) },
    '@/components/states': { EmptyState: 'denied', ErrorState: 'error' },
    '@/api/auth': { authApi: {} }, '@/lib/api': { asArray: value => value },
  }
  const loaded = new Map()
  function load(relative) {
    if (loaded.has(relative)) return loaded.get(relative)
    const source = fs.readFileSync(path.join(sourceRoot, relative), 'utf8')
    const compiled = ts.transpileModule(source, { compilerOptions: {
      module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.ReactJSX,
    } }).outputText
    const exports = {}; loaded.set(relative, exports)
    const execute = vm.runInThisContext(`(function(require,exports){${compiled}\n})`, { filename: relative })
    execute(name => name in deps ? deps[name] : name.startsWith('@/')
      ? load(`src/${name.slice(2)}.ts`) : require(name), exports)
    return exports
  }
  let result = load('src/components/layout/RoutePermissionGuard.tsx').RoutePermissionGuard({ children: 'authorized-content' })
  while (result && typeof result.type === 'function') result = result.type(result.props)
  return typeof result === 'string' ? result : result.type
}

test('a known positive host menu remains accessible when an unrelated application menu fails', () => {
  assert.equal(guard({ pathname: '/system/oauth/clients', menus: [
    { status: 'success', data: [menu('/system/oauth/clients')] }, { status: 'error' },
  ] }), 'authorized-content')
})

test('unknown routes are denied even if an authorized menu happens to name that path', () => {
  assert.equal(guard({ pathname: '/unregistered-admin', menus: [
    { status: 'success', data: [menu('/unregistered-admin')] },
  ] }), 'denied')
})

test('personal dashboard stays accessible without an application MENU or dashboard permission', () => {
  assert.equal(guard({ pathname: '/dashboard', permissions: [], menus: [{ status: 'error' }] }), 'authorized-content')
})

test('platform-root-only host pages remain denied in a platform tenant child context', () => {
  assert.equal(guard({ pathname: '/system/oauth/clients', platformContext: false, menus: [
    { status: 'success', data: [menu('/system/oauth/clients')] },
  ] }), 'denied')
})

test('a failed menu query cannot authorize a host page from its cached data', () => {
  assert.equal(guard({ pathname: '/system/oauth/clients', menus: [
    { status: 'error', data: [menu('/system/oauth/clients')] },
  ] }), 'error')
})

test('an embed URL with no exact returned MENU remains denied', () => {
  assert.equal(guard({ pathname: '/embed', search: '?url=https%3A%2F%2Fexample.test%2Fprivate', menus: [
    { status: 'success', data: [menu('/external', { linkUrl: 'https://example.test/public' })] },
  ] }), 'denied')
})
