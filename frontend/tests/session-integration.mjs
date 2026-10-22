import assert from 'node:assert/strict'
import test from 'node:test'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const sourceRoot = process.env.SESSION_INTEGRATION_SOURCE_ROOT ? path.resolve(root, process.env.SESSION_INTEGRATION_SOURCE_ROOT) : root
const require = createRequire(path.join(root, 'package.json'))
const ts = require('typescript')
const deferred = () => { let resolve; const promise = new Promise(r => { resolve = r }); return { promise, resolve } }
const source = relative => fs.readFileSync(path.join(sourceRoot, relative), 'utf8')

// Execute actual consumer closures with controlled collaborators, so a late
// callback can be held exactly across a competing login without timer races.
function expression(relative, name, bindings = {}, kind = 'variable') {
  const text = source(relative), ast = ts.createSourceFile(relative, text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX)
  let found
  function visit(node) {
    if (kind === 'variable' && ts.isVariableDeclaration(node) && node.name.getText(ast) === name) found = node.initializer.getText(ast)
    if (kind === 'function' && ts.isFunctionDeclaration(node) && node.name?.text === name) found = node.getText(ast).replace(/^export\s+/, '')
    if (kind === 'property' && ts.isPropertyAssignment(node) && node.name.getText(ast) === name) found = node.initializer.getText(ast)
    if (kind === 'runtime-effect' && ts.isCallExpression(node) && node.expression.getText(ast) === 'React.useEffect' && node.arguments[0]?.getText(ast).includes('new MicroContextRuntime')) found = node.arguments[0].getText(ast)
    if (kind === 'logout-selection' && ts.isJsxAttribute(node) && node.name.getText(ast) === 'onSelect' && node.initializer?.expression) {
      const value = node.initializer.expression.getText(ast)
      if (value.includes('logoutPlatformSession') || value.includes('authApi.logout')) found = value
    }
    ts.forEachChild(node, visit)
  }
  visit(ast)
  assert.ok(found, `Actual ${name} consumer must be present in ${relative}`)
  const output = ts.transpileModule(`const result = (${found});`, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } }).outputText
  return vm.runInNewContext(`${output}\nresult`, { DOMException, Promise, ...bindings }, { filename: relative })
}

test('a mounted iframe failure stops authorization renewal and exposes the host retry state', () => {
  const active = {}, events = []
  let retired = false
  const handle = expression('src/components/micro/MicroAppContainer.tsx', 'handleIframeFailure', {
    stale: () => retired, activeRef: { current: active }, active,
    reportRuntimeFailure: (...args) => events.push(['report', ...args]),
    runtimeRef: { current: { stop: () => events.push(['stop']) } },
    setStatus: status => events.push(['status', status]), setErrorMessage: message => events.push(['message', message]),
    disposeEntry: entry => { assert.equal(entry, active); events.push(['dispose']); retired = true },
  })
  handle(new Error('Iframe application navigated away from its connected document'))
  assert.deepEqual(events.map(([kind]) => kind), ['report', 'stop', 'status', 'message', 'dispose'])
  assert.deepEqual(events.find(([kind]) => kind === 'status'), ['status', 'error'])
  handle(new Error('late failure'))
  assert.equal(events.length, 5, 'a retired mounting instance cannot overwrite the replacement UI')
})

test('context selection delegates the whole operation to the locked platform transaction', async () => {
  const calls = [], expected = { accessToken: 'organization-b', refreshToken: 'refresh-b' }
  const useSwitch = expression('src/hooks/use-auth-data.ts', 'useSwitchContext', {
    switchPlatformContext: async id => { calls.push(id); return expected },
    useQueryClient: () => { throw new Error('Consumer must not create a second switching transaction') },
  }, 'function')
  assert.equal(await useSwitch()('org-b'), expected)
  assert.deepEqual(calls, ['org-b'])
})

function loginFixture(committed) {
  const events = [], pair = { accessToken: 'new-access', refreshToken: 'new-refresh', user: { username: 'new-user' } }
  const fence = { sessionId: 'previous-session', version: 4 }
  const action = expression('src/pages/login/LoginPage.tsx', 'selectContext', {
    captureSessionFence: () => fence,
    authApi: { selectContext: async () => pair },
    setSelecting: () => {}, setError: value => events.push(['error', value]), setStep: () => {}, setLoginData: () => {}, setSelectedTenantId: () => {},
    queryClient: { cancelQueries: async () => events.push(['cancel']), clear: () => events.push(['clear']) },
    setSession: async (...args) => { events.push(['commit', args]); return committed.promise },
    toast: { success: message => events.push(['welcome', message]) }, navigate: destination => events.push(['navigate', destination]), returnTo: '/dashboard',
    ApiError: class extends Error {},
  })
  return { action, events, fence, data: { ticket: 'credential-ticket', sessionFence: fence } }
}

test('login waits for the atomic session commit before navigation and welcome', async () => {
  const committed = deferred(), f = loginFixture(committed)
  const pending = f.action('org-b', f.data)
  await new Promise(resolve => setImmediate(resolve))
  assert.equal(f.events.some(([event]) => event === 'navigate' || event === 'welcome'), false)
  committed.resolve(true); await pending
  assert.equal(f.events.find(([event]) => event === 'commit')[1][2], f.fence)
  assert.equal(f.events.filter(([event]) => event === 'navigate').length, 1)
})

test('a superseded login selection cannot navigate or clear the replacement session', async () => {
  const committed = deferred(), f = loginFixture(committed)
  const pending = f.action('org-b', f.data)
  committed.resolve(false); await pending
  assert.equal(f.events.some(([event]) => ['navigate', 'welcome', 'clear'].includes(event)), false)
  assert.ok(f.events.some(([event, message]) => event === 'error' && message))
})

test('login captures its replacement fence before the first credential request', async () => {
  const pending = deferred(), fence = { sessionId: 'original-session', version: 3 }, selected = []
  let current = fence
  const submit = expression('src/pages/login/LoginPage.tsx', 'onSubmit', {
    captureSessionFence: () => current, setError: () => {},
    localStorage: { setItem() {}, removeItem() {} }, REMEMBER_KEY: 'remember',
    authApi: { login: () => pending.promise }, setLoginData: data => selected.push(data),
    setSelectedTenantId() {}, setSelectionSearch() {}, setStep() {},
  })
  const action = submit({ username: 'user', password: 'secret', remember: false })
  current = { sessionId: 'replacement-session', version: 6 }
  pending.resolve({ ticket: 'ticket', memberships: [{ organizationId: 'org' }] })
  await action
  assert.equal(selected[0].sessionFence, fence)
})

for (const [label, relative, name, kind] of [
  ['user menu', 'src/components/layout/TopBar.tsx', 'handleLogout', 'variable'],
  ['command palette', 'src/components/layout/CommandPalette.tsx', 'logout selection', 'logout-selection'],
]) test(`${label} cannot clear or redirect after a superseded logout`, async () => {
  const events = []
  const action = expression(relative, name, {
    logoutPlatformSession: async () => false,
    authApi: { logout: async () => events.push('legacy-request') }, logout: () => events.push('legacy-delete'),
    queryClient: { clear: () => events.push('clear') }, navigate: () => events.push('navigate'), onOpenChange: () => events.push('close'),
    toast: { success: () => events.push('success'), warning: () => events.push('warning'), error: () => events.push('error') },
  }, kind)
  await action(); await new Promise(resolve => setImmediate(resolve))
  assert.equal(events.some(event => ['clear', 'navigate', 'legacy-delete', 'success'].includes(event)), false)
})

test('password change captures its request fence and does not log out a later login', async () => {
  const events = [], fence = { sessionId: 'old-session', version: 2 }
  const mutation = expression('src/pages/profile/ProfilePage.tsx', 'changeMutation', {
    useMutation: options => options, captureSessionFence: () => fence,
    authApi: { changePassword: async (...args) => events.push(['password', args]) },
    useAuthStore: { getState: () => ({ logout: async expected => { events.push(['logout', expected]); return false } }) },
    logout: () => events.push(['legacy-delete']), form: { reset: () => events.push(['reset']) },
    queryClient: { clear: () => events.push(['clear']) }, navigate: () => events.push(['navigate']),
    toast: { success: () => events.push(['success']), error: () => {} },
  })
  const result = await mutation.mutationFn({ oldPassword: 'old', newPassword: 'new' })
  await mutation.onSuccess(result)
  assert.equal(events.find(([event]) => event === 'password')[1][2], fence)
  assert.equal(events.find(([event]) => event === 'logout')[1], fence)
  assert.equal(events.some(([event]) => ['legacy-delete', 'clear', 'navigate', 'success'].includes(event)), false)
})

test('password API forwards the captured session fence to the platform request', async () => {
  const calls = [], exports = {}, text = source('src/api/auth.ts')
  const output = ts.transpileModule(text, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } }).outputText
  vm.runInNewContext(`(function(require,exports){${output}\n})`, {})(() => ({ api: { put: (...args) => calls.push(args) } }), exports)
  const fence = { sessionId: 'old', version: 5 }
  exports.authApi.changePassword('old-password', 'new-password', fence)
  assert.equal(calls[0][2].sessionFence, fence)
})

function apiFixture(relative) {
  const calls = [], captured = [], fence = { sessionId: 'platform-session', version: 7 }, exports = {}
  const output = ts.transpileModule(source(relative), { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } }).outputText
  vm.runInNewContext(`(function(require,exports){${output}\n})`, {})(name => {
    if (name === '@/stores/auth') return { capturePlatformSessionFence: token => { captured.push(token); return fence } }
    if (name === '@/lib/api') return {
      api: Object.fromEntries(['get', 'post', 'put'].map(method => [method, (...args) => { calls.push({ method, args }); return {} }])),
      switchPlatformContext: (...args) => { calls.push({ method: 'switch', args }); return {} },
      logoutPlatformSession: async () => { calls.push({ method: 'logout-transaction', args: [] }); return true },
    }
    throw new Error(`Unexpected API dependency ${name}`)
  }, exports)
  return { calls, captured, fence, ...exports }
}

test('profile/menu/permission platform bearers opt into session fencing while login tickets stay independent', () => {
  const f = apiFixture('src/api/auth.ts')
  f.authApi.profile('platform-access')
  f.authApi.menus('application', 'platform-access')
  f.authApi.permissions('platform-access')
  for (const call of f.calls) assert.equal(call.args.at(-1).sessionFence, f.fence)
  assert.deepEqual(f.captured, ['platform-access', 'platform-access', 'platform-access'])
  f.authApi.selectContext('organization', 'credential-ticket')
  assert.equal(f.calls.at(-1).args.at(-1).sessionFence, undefined)
  assert.equal(f.captured.length, 3)
  f.authApi.profile()
  assert.equal(f.calls.at(-1).args.at(-1).sessionFence, undefined)
  f.authApi.switchContext('organization-b', 'platform-access')
  assert.equal(f.calls.at(-1).method, 'switch')
  assert.equal(f.calls.at(-1).args[1], f.fence)
  assert.equal(f.captured.at(-1), 'platform-access')
})

test('the compatible auth logout API delegates to the same coordinated logout transaction', async () => {
  const f = apiFixture('src/api/auth.ts'), result = await f.authApi.logout()
  assert.equal(result.ok, true)
  assert.equal(f.calls.length, 1)
  assert.equal(f.calls[0].method, 'logout-transaction')
})

test('session-token and platform settings consumers fence their captured platform credentials', () => {
  const oauth = apiFixture('src/api/oauth.ts'), platform = apiFixture('src/api/platform.ts')
  oauth.oauthApi.sessionToken('work-orders', 'platform-access')
  platform.platformApi.settings('platform-access')
  platform.platformApi.updateSettings({ platformName: 'Example' }, 'platform-access')
  for (const f of [oauth, platform]) {
    for (const call of f.calls) assert.equal(call.args.at(-1).sessionFence, f.fence)
    assert.ok(f.captured.every(token => token === 'platform-access'))
  }
  platform.platformApi.branding()
  assert.equal(platform.calls.at(-1).args.at(-1).sessionFence, undefined)
})

test('micro application-token issuance fences the platform bearer at its existing request boundary', async () => {
  const fence = { sessionId: 'session-a', version: 4 }, calls = [], signal = new AbortController().signal
  const issue = expression('src/components/micro/MicroAppContainer.tsx', 'issue', {
    appCode: 'work-orders', sessionId: 'session-a', contextEpoch: 3, sessionFence: fence,
    reconcileSessionSync() {}, useAuthStore: { getState: () => ({ token: 'platform-access', sessionId: 'session-a', contextEpoch: 3, contextSwitching: false }) },
    api: { post: (...args) => calls.push(args) },
  }, 'property')
  await issue(signal)
  assert.equal(calls[0][0], '/api/oauth/session-token')
  assert.equal(calls[0][2].signal, signal)
  assert.equal(calls[0][2].sessionFence, fence)
  assert.equal(calls[0][2].bearer, undefined)
})

test('the actual authorization effect accepts token rotation before React renders and rejects another logical context', async () => {
  const state = { token: 'old-access', sessionId: 'session-a', sessionVersion: 4, contextEpoch: 3, contextSwitching: false }, calls = []
  let captured
  const ref = () => ({ current: null }), setter = () => {}
  const effect = expression('src/components/micro/MicroAppContainer.tsx', 'authorization effect', {
    sessionId: 'session-a', contextEpoch: 3, sessionToken: 'old-access',
    useAuthStore: { getState: () => state }, reconcileSessionSync() {}, appCode: 'work-orders', isTrusted: false,
    snapshotRef: ref(), activeRef: ref(), lastPushSigRef: ref(), lastDeliveryRef: ref(), runtimeRef: ref(),
    setSnapshot: setter, setStatus: setter, setTokenState: setter, setTokenNote: setter, setErrorMessage: setter, setRecoveryAttempt: setter,
    assertBrowserCapabilities() {}, MicroContextRuntime: class { constructor(options) { captured = options } refresh() { return Promise.resolve() } stop() {} },
    api: { post: (...args) => { calls.push({ args, latestToken: state.token }); return Promise.resolve({ accessToken: 'application-token' }) } },
  }, 'runtime-effect')
  const cleanup = effect()
  state.token = 'rotated-access'; state.sessionVersion++
  await captured.issue(new AbortController().signal)
  assert.equal(calls[0].latestToken, 'rotated-access')
  assert.equal(calls[0].args[2].bearer, undefined, 'A render-time bearer must not survive a rotation')
  assert.equal(calls[0].args[2].sessionFence.sessionId, 'session-a')
  state.sessionId = 'session-b'
  assert.throws(() => captured.issue(new AbortController().signal), error => error.name === 'AbortError')
  state.sessionId = 'session-a'; state.contextEpoch++
  assert.throws(() => captured.issue(new AbortController().signal), error => error.name === 'AbortError')
  assert.equal(calls.length, 1)
  cleanup()
})

test('socket rejection preserves same-context HTTP fallback while retired contexts cancel their old queries', () => {
  for (const [current, replacementKind, cancel] of [[true, 'none', false], [false, 'none', true], [false, 'same-key', false], [false, 'new-context', true]]) {
    const events = [], controller = new AbortController()
    const key = ['notifications', 'user', 'tenant', 'organization', 3]
    const stop = expression('src/hooks/use-notification-socket.tsx', 'stop', {
      stopped: false, timers() {}, authenticated: {}, stop() {},
      captured: { signal: controller.signal, current: () => current, key },
      getNotificationContext: () => replacementKind === 'none' ? null : { key: replacementKind === 'same-key' ? [...key] : [...key.slice(0, -1), 4] },
      socket: { io: { reconnection: () => events.push('reconnection-off') }, removeAllListeners: () => events.push('remove-listeners'), disconnect: () => events.push('disconnect') },
      toastIds: new Set(), seenIds: new Set(), toast: { dismiss() {} },
      queryClient: { cancelQueries: () => events.push('cancel-http') }, disconnected: () => events.push('disconnected'),
    })
    stop(); stop()
    assert.equal(events.filter(event => event === 'disconnect').length, 1)
    assert.equal(events.includes('cancel-http'), cancel)
  }
})

test('notification requests retain both their context lifetime and their captured platform session fence', () => {
  const fence = { sessionId: 'session-a', version: 4 }, controller = new AbortController(), caller = new AbortController()
  const options = expression('src/hooks/use-notification-context.ts', 'notificationRequestOptions', {
    AbortSignal,
    capturePlatformSessionFence: token => { assert.equal(token, 'platform-access'); return fence },
  }, 'function')
  const scope = { token: 'platform-access', signal: controller.signal, userId: 'user', tenantId: 'tenant', organizationId: 'org', current: () => true }
  const result = options(scope, caller.signal)
  assert.equal(result.sessionFence, fence)
  assert.equal(result.current, scope.current)
  controller.abort()
  assert.equal(result.signal.aborted, true)
})

function gateFixture(status, pathname = '/system/users') {
  const state = { sessionStatus: status, coordinationError: '会话状态待确认', sessionId: 'session-a', contextEpoch: 3 }, exports = {}
  const text = source('src/components/session-status.tsx')
  const output = ts.transpileModule(text, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX } }).outputText
  const jsx = (type, props, key) => ({ type, props, key })
  vm.runInNewContext(`(function(require,exports){${output}\n})`, { window: { location: { pathname } } })(name => {
    if (name === '@/stores/auth') return { useAuthStore: select => select(state) }
    if (name === 'react/jsx-runtime') return { jsx, jsxs: jsx, Fragment: 'Fragment' }
    if (name === 'react') return { Fragment: 'Fragment' }
    throw new Error(`Unexpected gate dependency: ${name}`)
  }, exports)
  return { state, render: () => exports.SessionStatusGate({ children: 'private-view' }) }
}

test('missing coordination capabilities show a clear blocking prompt before private components mount', () => {
  const gate = gateFixture('unavailable'), result = gate.render(), text = JSON.stringify(result)
  assert.equal(text.includes('private-view'), false)
  assert.match(text, /浏览器|存储|安全/)
})

test('switching retires private children without mounting their anonymous-login redirect', () => {
  const result = gateFixture('switching').render(), text = JSON.stringify(result)
  assert.equal(text.includes('private-view'), false)
  assert.equal(text.includes('/login'), false)
  assert.match(text, /正在同步登录与组织/)
})

test('unknown outcomes hide private views and allow an explicit login recovery page', () => {
  assert.equal(JSON.stringify(gateFixture('unknown').render()).includes('private-view'), false)
  assert.equal(gateFixture('unknown', '/login').render().props.children, 'private-view')
})

test('the session boundary retires all old host drafts on epoch change but preserves ordinary refresh state', () => {
  const gate = gateFixture('active'), initial = gate.render()
  gate.state.sessionVersion = 8
  assert.equal(gate.render().key, initial.key)
  gate.state.contextEpoch++
  assert.notEqual(gate.render().key, initial.key)
  assert.equal(gate.render().props.children, 'private-view')
})

test('a socket callback reconciles shared state and synchronously aborts its old notification lifetime', () => {
  let state = { token: 'old', contextEpoch: 3, contextSwitching: false, profile: { user: { id: 'user' }, tenant: { id: 'tenant' }, organization: { id: 'org-a' } } }
  let shared = state
  const listeners = new Set(), cache = new Map(), sandbox = vm.createContext({ DOMException, AbortController })
  const useAuthStore = { getState: () => state, subscribe: listener => { listeners.add(listener); return () => listeners.delete(listener) } }
  const reconcileSessionSync = () => { if (state !== shared) { state = shared; for (const listener of [...listeners]) listener() } }
  function load(relative) {
    if (cache.has(relative)) return cache.get(relative)
    const exports = {}, text = source(relative).replace(/^if \(import\.meta\.hot\).*$/m, '')
    const output = ts.transpileModule(text, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } }).outputText
    cache.set(relative, exports)
    vm.runInContext(`(function(require,exports){${output}\n})`, sandbox)(name => {
      if (name === '@/stores/auth') return { useAuthStore, reconcileSessionSync }
      if (name === 'react') return { useSyncExternalStore() {} }
      if (name === '@/lib/notification-context') return load('src/lib/notification-context.ts')
      throw new Error(`Unexpected notification boundary dependency ${name}`)
    }, exports)
    return exports
  }
  const boundary = load('src/hooks/use-notification-context.ts'), old = boundary.getNotificationContext()
  let stopped = false
  old.signal.addEventListener('abort', () => { stopped = true })
  shared = { ...state, token: 'new', contextEpoch: 4, profile: { ...state.profile, organization: { id: 'org-b' } } }
  // No storage/BC event has been delivered. The old callback itself checks.
  assert.equal(stopped, false)
  assert.equal(old.current(), false)
  assert.equal(stopped, true)
  assert.equal(old.signal.aborted, true)
  assert.equal(boundary.getNotificationContext().organizationId, 'org-b')
})

test('bootstrap initializes coordination before render and clears cache synchronously on epoch changes', () => {
  const events = [], exports = {}, text = source('src/main.tsx')
  let listener
  const output = ts.transpileModule(text, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX } }).outputText
  const jsx = (type, props, key) => ({ type, props, key })
  vm.runInNewContext(`(function(require,exports){${output}\n})`, { document: { getElementById: () => ({}) } })(name => {
    if (name === '@/stores/auth') return { authStore: { subscribe: value => { listener = value }, getState: () => ({ initializeSessionCoordination: () => { events.push('initialize'); return Promise.resolve() } }) } }
    if (name === 'react') return { default: { StrictMode: 'StrictMode' } }
    if (name === 'react-dom/client') return { default: { createRoot: () => ({ render: () => events.push('render') }) } }
    if (name === 'react/jsx-runtime') return { jsx, jsxs: jsx }
    if (name === '@tanstack/react-query') return { QueryClient: class { cancelQueries() { events.push('cancel'); return Promise.resolve() } clear() { events.push('clear') } }, QueryClientProvider: 'QueryClientProvider' }
    return {}
  }, exports)
  assert.deepEqual(events, ['initialize', 'render'])
  listener({ contextEpoch: 4 }, { contextEpoch: 3 })
  assert.deepEqual(events.slice(-2), ['cancel', 'clear'])
  const count = events.length
  listener({ contextEpoch: 4 }, { contextEpoch: 4 })
  assert.equal(events.length, count)
})
