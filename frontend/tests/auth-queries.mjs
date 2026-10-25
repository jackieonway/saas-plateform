import assert from 'node:assert/strict'
import test from 'node:test'
import fs from 'node:fs'
import vm from 'node:vm'
import path from 'node:path'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const sourceRoot = process.env.AUTH_QUERY_SOURCE_ROOT ? path.resolve(root, process.env.AUTH_QUERY_SOURCE_ROOT) : root
const require = createRequire(path.join(root, 'package.json'))
const ts = require('typescript')
const { QueryClient } = require('@tanstack/react-query')
const deferred = () => { let resolve; const promise = new Promise(r => { resolve = r }); return { promise, resolve } }
const node = (id, extra = {}) => ({ id, parentId: null, type: 'MENU', title: id, path: `/micro/app-a/${id}`, sortOrder: 0, visible: true, permission: null, icon: null, ...extra })
const id = (value) => `01962345-6789-7abc-8def-${value.toString(16).padStart(12, '0')}`
const identity = () => ({ userId: id(1), tenantId: id(2), organizationId: id(3), membershipId: id(4), memberType: 'EMPLOYEE', identityVersion: '1', accountVersion: '0' })
const namespace = (proof) => JSON.stringify([proof.userId, proof.tenantId, proof.organizationId, proof.membershipId, proof.memberType, proof.identityVersion, proof.accountVersion])
const profile = (proof = identity()) => ({ user: { id: proof.userId }, tenant: { id: proof.tenantId }, organization: { id: proof.organizationId },
  membershipId: proof.membershipId, memberType: proof.memberType, identityVersion: proof.identityVersion, accountVersion: proof.accountVersion,
  applications: [{ id: 'app-a', code: 'app-a', basePath: '/micro/app-a' }, { id: 'app-b', code: 'app-b', basePath: '/micro/app-b' }], memberships: [] })

function fixture(handler = async () => []) {
  const proof = identity()
  const state = { token: 'session-a', contextEpoch: 1, contextSwitching: false, identity: proof, contextKey: namespace(proof), profile: profile(proof) }
  const calls = [], observed = { query: null, queries: [], results: [] }
  const store = selector => selector(state)
  store.getState = () => state
  const cache = new Map()
  const sandbox = vm.createContext({ DOMException, AbortController, AbortSignal, Promise, Set, URL, URLSearchParams })
  function load(relative) {
    if (cache.has(relative)) return cache.get(relative)
    const source = fs.readFileSync(path.join(sourceRoot, relative), 'utf8')
    const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText
    const exports = {}
    cache.set(relative, exports)
    new vm.Script(`(function(require,exports){${compiled}\n})`, { filename: relative }).runInContext(sandbox)(name => {
      if (name === '@/stores/auth') return { useAuthStore: store }
      if (name === '@/api/auth') return { authApi: Object.fromEntries(['menus', 'profile', 'permissions'].map(method => [method, (...args) => { calls.push({ method, args }); return handler(method, ...args) }])) }
      if (name === '@/lib/api') return { asArray: (data, key) => Array.isArray(data) ? data : Array.isArray(data?.[key]) ? data[key] : [],
        switchPlatformContext: (...args) => { calls.push({ method: 'switch', args }); return Promise.resolve() } }
      if (name === '@tanstack/react-query') return { ...require(name), useQuery: options => { observed.query = options; return { ...options, status: 'success', data: { permissions: ['system:user'] } } }, useQueryClient: () => null, useQueries: ({ queries }) => { observed.queries = queries; return queries.map((_, i) => observed.results[i] ?? { status: 'pending', isPending: true, isSuccess: false, isError: false, dataUpdatedAt: 0 }) } }
      if (name === 'react') return { useMemo: callback => callback(), useCallback: callback => callback }
      if (name.startsWith('@/')) return load(`src/${name.slice(2)}.ts`)
      return require(name)
    }, exports)
    return exports
  }
  const hooks = load('src/hooks/use-auth-data.ts')
  const allMenus = () => {
    if (hooks.useApplicationMenusQueries) return hooks.useApplicationMenusQueries()
    return load('src/hooks/use-authorized-paths.ts').useAuthorizedPaths()
  }
  return { state, calls, observed, hooks, allMenus, load, factory: () => load('src/lib/auth-queries.ts') }
}

test('sidebar and all-application menus use one epoch-scoped query key', () => {
  const f = fixture()
  f.hooks.useMenusQuery('app-a')
  const single = f.observed.query
  f.allMenus()
  assert.deepEqual([...single.queryKey], [...f.observed.queries[0].queryKey])
  assert.deepEqual([...single.queryKey], ['auth-menus', 'app-a', f.state.contextKey, 1])
  assert.equal(f.observed.queries[0].staleTime, single.staleTime)
  assert.equal(f.observed.queries[0].refetchInterval, single.refetchInterval)
})

test('all-application menu query forwards the supplied cancellation signal', async () => {
  const f = fixture(), controller = new AbortController()
  f.allMenus()
  await f.observed.queries[0].queryFn({ signal: controller.signal })
  assert.equal(f.calls[0].args[2], controller.signal)
})

test('menu response that ignores transport abort is rejected after epoch change', async () => {
  const wait = deferred(), f = fixture(() => wait.promise)
  f.hooks.useMenusQuery('app-a')
  const pending = f.observed.query.queryFn({ signal: new AbortController().signal })
  f.state.contextEpoch++
  wait.resolve([node('old')])
  await assert.rejects(pending, error => error.name === 'AbortError')
})

test('shared menu factory deduplicates concurrent fetches and normalizes sorted trees', async () => {
  const wait = deferred(), f = fixture(() => wait.promise), factory = f.factory()
  const client = new QueryClient(), session = factory.captureAuthSession()
  const first = client.fetchQuery(factory.authMenusQueryOptions('app-a', session))
  const second = client.fetchQuery(factory.authMenusQueryOptions('app-a', session))
  wait.resolve({ menus: [node('last', { sortOrder: 9 }), node('first', { sortOrder: 1, children: [node('z', { sortOrder: 3 }), node('a', { sortOrder: 1 })] })] })
  const [a, b] = await Promise.all([first, second])
  assert.equal(f.calls.length, 1)
  assert.equal(a, b)
  assert.deepEqual(Array.from(a, item => item.id), ['first', 'last'])
  assert.deepEqual(Array.from(a[0].children, item => item.id), ['a', 'z'])
  client.clear()
})

test('query cancellation aborts the actual menu signal and cannot cache a late response', async () => {
  const wait = deferred(), f = fixture(() => wait.promise), factory = f.factory(), client = new QueryClient()
  const options = factory.authMenusQueryOptions('app-a', factory.captureAuthSession())
  const pending = client.fetchQuery(options)
  const rejection = assert.rejects(pending)
  const signal = f.calls[0].args[2]
  await client.cancelQueries()
  assert.equal(signal.aborted, true)
  wait.resolve([node('late')])
  await rejection
  assert.equal(client.getQueryData(options.queryKey), undefined)
  client.clear()
})

test('profile, permission and menu reads reject changed identity and switching', async () => {
  for (const kind of ['profile', 'permissions', 'menus']) {
    const wait = deferred(), f = fixture(() => wait.promise), factory = f.factory(), session = factory.captureAuthSession()
    const options = kind === 'menus' ? factory.authMenusQueryOptions('app-a', session) : factory[kind === 'profile' ? 'authProfileQueryOptions' : 'authPermissionsQueryOptions'](session)
    const pending = options.queryFn({ signal: new AbortController().signal })
    f.state.profile.organization.id = id(30)
    wait.resolve(kind === 'menus' ? [] : {})
    await assert.rejects(pending, error => error.name === 'AbortError')
  }
  const f = fixture(), factory = f.factory(), session = factory.captureAuthSession()
  f.state.contextSwitching = true
  await assert.rejects(factory.authMenusQueryOptions('app-a', session).queryFn({ signal: new AbortController().signal }), error => error.name === 'AbortError')
  assert.equal(f.calls.length, 0)
})

test('revoking an application rejects its in-flight menu result', async () => {
  const wait = deferred(), f = fixture(() => wait.promise), factory = f.factory()
  const pending = factory.authMenusQueryOptions('app-a', factory.captureAuthSession()).queryFn({ signal: new AbortController().signal })
  f.state.profile.applications.splice(0, 1)
  wait.resolve([node('revoked')])
  await assert.rejects(pending, error => error.name === 'AbortError')
})

test('same-epoch token rotation preserves menu cache identity and a valid response', async () => {
  const wait = deferred(), f = fixture(() => wait.promise), factory = f.factory(), first = factory.captureAuthSession()
  const options = factory.authMenusQueryOptions('app-a', first)
  const pending = options.queryFn({ signal: new AbortController().signal })
  f.state.token = 'rotated-session-a'
  assert.equal(factory.isCurrentAuthSession(first), true)
  assert.deepEqual([...options.queryKey], [...factory.authMenusQueryOptions('app-a', factory.captureAuthSession()).queryKey])
  wait.resolve([node('valid')])
  assert.equal((await pending)[0].id, 'valid')
})

test('all auth query options share bounded freshness and do not retry cancellation or denial', () => {
  const f = fixture(), factory = f.factory(), session = factory.captureAuthSession()
  for (const options of [factory.authProfileQueryOptions(session), factory.authPermissionsQueryOptions(session), factory.authMenusQueryOptions('app-a', session)]) {
    assert.equal(options.staleTime, 20_000)
    assert.equal(options.refetchInterval, 20_000)
    assert.equal(options.retry(0, new DOMException('cancelled', 'AbortError')), false)
    assert.equal(options.retry(0, { status: 403 }), false)
    assert.equal(options.retry(0, { status: 503 }), true)
    assert.equal(options.retry(1, { status: 503 }), false)
  }
})

test('disabled, logged-out and switching consumers do not enable auth requests', () => {
  const f = fixture()
  f.hooks.useMenusQuery('app-a', { enabled: false }); assert.equal(f.observed.query.enabled, false)
  f.hooks.usePermissionsQuery({ enabled: false }); assert.equal(f.observed.query.enabled, false)
  f.hooks.useApplicationMenusQueries({ enabled: false }); assert.ok(f.observed.queries.every(options => !options.enabled))
  f.state.contextSwitching = true
  f.hooks.useProfileQuery(); assert.equal(f.observed.query.enabled, false)
  f.state.contextSwitching = false; f.state.token = null
  f.hooks.useMenusQuery('app-a'); assert.equal(f.observed.query.enabled, false)
})

test('current-application query selection does not request unrelated menus', () => {
  const f = fixture()
  const selected = f.hooks.useApplicationMenusQueries({ applicationIds: ['app-a', 'unknown-app'] })
  assert.equal(selected.queries.length, 1)
  assert.equal(selected.queries[0].applicationId, 'app-a')
  assert.equal(f.observed.queries.length, 1)
  assert.deepEqual([...f.observed.queries[0].queryKey], ['auth-menus', 'app-a', f.state.contextKey, 1])
})

test('known positive path is available despite unrelated pending or failed menu query', () => {
  const f = fixture()
  const authorized = f.load('src/hooks/use-authorized-paths.ts')
  for (const status of ['pending', 'error']) {
    f.observed.results = [
      { status: 'success', isSuccess: true, dataUpdatedAt: 0, data: [node('users', { path: '/system/users' })] },
      { status, isPending: status === 'pending', isError: status === 'error', dataUpdatedAt: 0 },
    ]
    const result = authorized.useAuthorizedPaths()
    assert.equal(result.allowedPaths.has('/system/users'), true)
    assert.equal(result.access('/system/users').status, 'allowed')
    assert.notEqual(result.access('/system/roles').status, 'allowed')
  }
})

test('same-timestamp failure, application removal and switching remove old authorized paths', () => {
  const f = fixture(), authorized = f.load('src/hooks/use-authorized-paths.ts')
  const cached = { status: 'success', isSuccess: true, dataUpdatedAt: 42, data: [node('users', { path: '/system/users' })] }
  f.observed.results = [cached]
  assert.equal(authorized.useAuthorizedPaths().allowedPaths.has('/system/users'), true)
  f.observed.results[0] = { ...cached, status: 'error', isError: true, isSuccess: false }
  assert.equal(authorized.useAuthorizedPaths({ enabled: false }).allowedPaths.has('/system/users'), false)
  f.observed.results[0] = cached
  f.state.profile.applications = []
  assert.equal(authorized.useAuthorizedPaths().allowedPaths.has('/system/users'), false)
  f.state.contextSwitching = true
  assert.equal(authorized.useAuthorizedPaths({ enabled: false }).allowedPaths.size, 0)
  f.state.profile = null
  assert.equal(authorized.useAuthorizedPaths().allowedPaths.size, 0)
})

test('profile can load from a bound proof before profile exists, while menus and permissions wait', async () => {
  const f = fixture(async () => profile()), factory = f.factory()
  f.state.profile = null
  const session = factory.captureAuthSession(), client = new QueryClient()
  const options = factory.authProfileQueryOptions(session)
  assert.equal(options.enabled, true)
  assert.deepEqual([...options.queryKey], ['profile', f.state.contextKey, 1])
  assert.equal(factory.authMenusQueryOptions('app-a', session).enabled, false)
  assert.equal(factory.authPermissionsQueryOptions(session).enabled, false)
  const result = await client.fetchQuery(options)
  assert.equal(result.membershipId, f.state.identity.membershipId)
  assert.equal(client.getQueryData(options.queryKey), result)
  client.clear()
})

test('mismatched, incomplete and noncanonical profile proofs never enter the authorization cache', async () => {
  for (const patch of [{ membershipId: id(40) }, { memberType: 'CUSTOMER' }, { identityVersion: '2' },
    { accountVersion: '1' }, { membershipId: undefined }, { identityVersion: 1 }, { identityVersion: '01' },
    { accountVersion: '00' }, { accountVersion: '-1' }, { identityVersion: '9223372036854775808' }]) {
    const f = fixture(async () => ({ ...profile(), ...patch })), factory = f.factory(), client = new QueryClient()
    f.state.profile = null
    const options = factory.authProfileQueryOptions(factory.captureAuthSession())
    await assert.rejects(client.fetchQuery(options), error => error.name === 'AbortError')
    assert.equal(client.getQueryData(options.queryKey), undefined)
    client.clear()
  }
})

test('every auth cache key and pending read is fenced by concrete member, type and both versions', async () => {
  for (const patch of [{ membershipId: id(40) }, { memberType: 'CUSTOMER' }, { identityVersion: '2' }, { accountVersion: '1' }]) {
    for (const kind of ['profile', 'permissions', 'menus']) {
      const wait = deferred(), f = fixture(() => wait.promise), factory = f.factory()
      const original = factory.captureAuthSession()
      const query = session => kind === 'menus' ? factory.authMenusQueryOptions('app-a', session) :
        factory[kind === 'profile' ? 'authProfileQueryOptions' : 'authPermissionsQueryOptions'](session)
      const options = query(original), pending = options.queryFn({ signal: new AbortController().signal })
      const replacement = { ...f.state.identity, ...patch }
      f.state.identity = replacement; f.state.contextKey = namespace(replacement); f.state.profile = profile(replacement)
      assert.equal(factory.isCurrentAuthSession(original), false)
      assert.notDeepEqual([...options.queryKey], [...query(factory.captureAuthSession()).queryKey])
      wait.resolve(kind === 'profile' ? profile() : kind === 'menus' ? [node('old-member')] : { permissions: ['system:user'] })
      await assert.rejects(pending, error => error.name === 'AbortError')
    }
  }
})

test('missing or inconsistent bound proof never falls back to user and organization', async () => {
  for (const change of [state => { state.identity = null }, state => { state.contextKey = null },
    state => { state.contextKey = 'user-and-org-only' }, state => { state.identity = { ...state.identity, identityVersion: '01' } },
    state => { state.identity = { ...state.identity, memberType: 'CUSTOMER' }; state.contextKey = namespace(state.identity); state.profile = profile(state.identity) }]) {
    const f = fixture(), factory = f.factory()
    change(f.state)
    const session = factory.captureAuthSession()
    for (const options of [factory.authProfileQueryOptions(session), factory.authPermissionsQueryOptions(session), factory.authMenusQueryOptions('app-a', session)]) {
      assert.equal(options.enabled, false)
      await assert.rejects(options.queryFn({ signal: new AbortController().signal }), error => error.name === 'AbortError')
    }
    assert.equal(f.calls.length, 0)
  }
})

test('a stale profile cannot enable employee menus or permissions and large versions remain exact strings', () => {
  const f = fixture(), factory = f.factory()
  f.state.profile.membershipId = id(40)
  const mismatch = factory.captureAuthSession()
  assert.equal(factory.authProfileQueryOptions(mismatch).enabled, true)
  assert.equal(factory.authMenusQueryOptions('app-a', mismatch).enabled, false)
  assert.equal(factory.authPermissionsQueryOptions(mismatch).enabled, false)
  const proof = { ...identity(), identityVersion: '9007199254740993', accountVersion: '9223372036854775807' }
  f.state.identity = proof; f.state.contextKey = namespace(proof); f.state.profile = profile(proof)
  const exact = factory.captureAuthSession()
  assert.equal(exact.profileReady, true)
  assert.equal(exact.identity.identityVersion, '9007199254740993')
  assert.equal(exact.identity.accountVersion, '9223372036854775807')
  assert.equal(exact.namespace, namespace(proof))
  assert.equal(factory.authPermissionsQueryOptions(exact).enabled, true)
})

test('context switching submits the selected membership alongside its organization', async () => {
  const f = fixture()
  await f.hooks.useSwitchContext()(id(3), id(4))
  assert.deepEqual(f.calls, [{ method: 'switch', args: [id(3), id(4)] }])
})
