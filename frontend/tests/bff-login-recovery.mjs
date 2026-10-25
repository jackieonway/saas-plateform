import assert from 'node:assert/strict'
import test from 'node:test'
import fs from 'node:fs'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import { componentFixture, elements } from './helpers/component-fixture.mjs'
import { deferred, fixture, identity, json } from './helpers/session-fixture.mjs'
import * as contextIdentity from '../src/lib/context-identity.ts'

const require = createRequire(import.meta.url)
const ts = require('typescript')
const sid = '019d0000-0000-7000-8000-000000000010'
const proof = identity()
const csrf = 'd'.repeat(64)
const member = { userId: proof.userId, tenantId: proof.tenantId, organizationId: proof.organizationId,
  membershipId: proof.membershipId, memberType: 'EMPLOYEE', identityVersion: proof.identityVersion,
  status: 'ENABLED', consoleSelectable: true, selectionDisabledReason: null,
  tenantName: 'Owned recovery tenant', organizationName: 'Owned recovery organization', organizationPath: '/Owned recovery' }
const account = { id: proof.userId, username: 'owned-recovery-account', realName: 'Owned recovery account', avatar: '' }
const settle = () => new Promise(resolve => setImmediate(resolve))
const text = tree => Array.isArray(tree) ? tree.map(text).join('') : tree && typeof tree === 'object'
  ? text(tree.props?.children) : typeof tree === 'string' || typeof tree === 'number' ? String(tree) : ''

function freeze(value) {
  if (value && typeof value === 'object') {
    Object.values(value).forEach(freeze)
    Object.freeze(value)
  }
  return value
}

/** Run the real gate in a browser-shaped VM without changing Node globals. */
function sessionGate(authState, pathname = '/login') {
  const source = fs.readFileSync(new URL('../src/components/session-status.tsx', import.meta.url), 'utf8')
  const compiled = ts.transpileModule(source, { compilerOptions: {
    module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.ReactJSX,
  } }).outputText
  const jsx = (type, props, key) => ({ type, props, key })
  const dependencies = {
    react: { Fragment: 'Fragment' },
    'react/jsx-runtime': { jsx, jsxs: jsx },
    '@/stores/auth': { useAuthStore: selector => selector(authState) },
  }
  const sandbox = vm.createContext({ window: { location: { pathname } } })
  const exports = {}
  new vm.Script(`(function(require,exports){${compiled}\n})`, { filename: 'src/components/session-status.tsx' })
    .runInContext(sandbox)(name => {
      assert.ok(Object.hasOwn(dependencies, name), `Unexpected session gate dependency: ${name}`)
      return dependencies[name]
    }, exports)
  return children => exports.SessionStatusGate({ children })
}

function recoveryPage(authState, { login = () => {}, reload = async () => {} } = {}) {
  const calls = { login: 0, reload: 0, select: [], logout: [] }
  const forbiddenAuthority = new Proxy({}, { get: () => { throw new Error('The login view must delegate authentication authority to its callbacks') } })
  const component = componentFixture('src/pages/login/BffLoginPage.tsx', ['BffLoginPage'], {
    '@/hooks/use-platform-branding': { usePlatformBranding: () => ({
      platformName: 'Owned recovery platform', platformDescription: 'Owned recovery fixture', footerText: '',
    }) },
    '@/hooks/use-document-title': { useDocumentTitle() {} },
    '@/lib/context-identity': contextIdentity,
    '@/lib/bff-session': forbiddenAuthority,
    '@/stores/auth': forbiddenAuthority,
    '@/lib/api': forbiddenAuthority,
    '@/api/auth': forbiddenAuthority,
  })
  const props = {
    session: freeze(authState.bffSession), returnTo: '/system/users',
    onLogin: () => { calls.login++; return login() },
    onReload: async () => { calls.reload++; await reload() },
    onSelect: async value => { calls.select.push(value) },
    onLogout: async scope => { calls.logout.push(scope) },
  }
  const gate = sessionGate(authState)
  return { calls, unmount: () => component.unmount(),
    render: () => gate(component.render('BffLoginPage', props)),
  }
}

function button(tree, label) {
  const matches = elements(tree, element => element.type === 'Button' && text(element) === label)
  assert.equal(matches.length, 1, `The real recovery view must show exactly one ${label} button`)
  return matches[0]
}

async function authenticated(t, { memberless = false } = {}) {
  let outage = false
  const view = { sessionId: sid, version: '1', csrfToken: csrf, account,
    memberships: memberless ? [] : [member], context: memberless ? null : proof,
    expiresAt: new Date(Date.now() + 600_000).toISOString() }
  const bridge = { sessionId: sid, version: '1', accessToken: 'owned-recovery-memory-token', expiresIn: 300,
    user: { ...account, email: '', phone: '', gender: '', isSuperAdmin: false },
    tenant: { id: proof.tenantId, name: member.tenantName, code: 'owned' },
    organization: { id: proof.organizationId, name: member.organizationName, path: member.organizationPath },
    membershipId: proof.membershipId, memberType: 'EMPLOYEE', identityVersion: proof.identityVersion, accountVersion: proof.accountVersion }
  const auth = await fixture(async url => {
    if (url === '/auth/session') return outage ? json(503, {}) : json(200, view)
    if (url === '/auth/context-token') return json(200, bridge)
    throw new Error(`Recovery must not automatically select, log out or call another endpoint: ${url}`)
  }, { authMode: 'BFF', session: false, controlDeadline: true })
  t.after(() => auth.load('src/lib/bff-session.ts').bffSessionCoordinator.dispose())
  return { auth, failAuthority: () => { outage = true } }
}

function authoritySnapshot(auth) {
  const state = auth.state()
  return JSON.stringify({ authMode: state.authMode, bffSession: state.bffSession,
    token: state.token, refreshToken: state.refreshToken, sessionId: state.sessionId,
    sessionStatus: state.sessionStatus, contextEpoch: state.contextEpoch, storage: [...auth.storage] })
}

test('a real GET 503 retains an unknown account view with an explicit reauthentication path through the login gate', async t => {
  const { auth, failAuthority } = await authenticated(t)
  assert.equal(auth.state().sessionStatus, 'active')
  assert.equal(auth.state().token, 'owned-recovery-memory-token')
  failAuthority()
  await assert.rejects(auth.state().reconcileSession(), error => error.status === 503)
  assert.equal(auth.state().sessionStatus, 'unknown')
  assert.equal(auth.state().token, null)
  assert.equal(auth.state().bffSession.sessionId, sid)

  const opening = deferred()
  const page = recoveryPage(auth.state(), { login: () => opening.promise, reload: () => auth.state().reconcileSession() })
  t.after(() => { opening.resolve(); page.unmount() })
  let tree = page.render()
  assert.equal(button(tree, '重新确认账号').props.disabled, false)
  assert.equal(elements(tree, element => element.props?.['data-testid'] === `bff-membership-${proof.membershipId}`).length, 1)
  assert.deepEqual(page.calls, { login: 0, reload: 0, select: [], logout: [] })

  // A failed explicit GET must leave the independent login intent available.
  button(tree, '重新检查').props.onClick()
  await settle()
  tree = page.render()
  assert.equal(page.calls.reload, 1)
  assert.ok(text(tree).includes('操作未完成'))
  const confirm = button(tree, '重新确认账号')
  assert.equal(confirm.props.disabled, false)
  const requests = auth.calls.length, snapshot = authoritySnapshot(auth)
  confirm.props.onClick(); confirm.props.onClick()
  assert.deepEqual(page.calls, { login: 1, reload: 1, select: [], logout: [] })
  tree = page.render()
  assert.equal(button(tree, '重新确认账号').props.disabled, true)
  assert.ok(text(tree).includes('正在打开统一登录'))
  assert.equal(auth.calls.length, requests, 'Choosing reauthentication must not replay a bridge or logout command')
  assert.equal(authoritySnapshot(auth), snapshot, 'The view must retain server authority while delegating the explicit login intent')
  opening.resolve(); await settle()
  assert.equal(button(page.render(), '重新确认账号').props.disabled, false)
  assert.equal(page.calls.login, 1)
})

for (const memberless of [false, true]) test(`an active ${memberless ? 'memberless' : 'employee'} account can explicitly reconfirm without automatic commands`, async t => {
  const { auth } = await authenticated(t, { memberless })
  const opening = deferred()
  const page = recoveryPage(auth.state(), { login: () => opening.promise })
  t.after(() => { opening.resolve(); page.unmount() })
  const snapshot = authoritySnapshot(auth), requests = auth.calls.length
  let tree = page.render()
  assert.equal(auth.state().sessionStatus, 'active')
  assert.equal(auth.state().bffSession.memberships.length, memberless ? 0 : 1)
  assert.ok(text(tree).includes(memberless ? '暂无有效员工身份' : '选择本次员工身份'))
  assert.deepEqual(page.calls, { login: 0, reload: 0, select: [], logout: [] })
  const confirm = button(tree, '重新确认账号')
  assert.equal(confirm.props.disabled, false)
  confirm.props.onClick(); confirm.props.onClick()
  assert.deepEqual(page.calls, { login: 1, reload: 0, select: [], logout: [] })
  tree = page.render()
  assert.equal(button(tree, '重新确认账号').props.disabled, true)
  assert.ok(text(tree).includes('正在打开统一登录'))
  assert.equal(auth.calls.length, requests)
  assert.equal(authoritySnapshot(auth), snapshot)
  opening.resolve(); await settle()
  assert.equal(button(page.render(), '重新确认账号').props.disabled, false)
})

test('a fresh BFF GET 503 still permits explicit unified login through the real unavailable login gate', async t => {
  const auth = await fixture(async url => {
    assert.equal(url, '/auth/session', 'Fresh recovery must not automatically bridge, select or log out')
    return json(503, {})
  }, { authMode: 'BFF', session: false, initialize: false, controlDeadline: true })
  t.after(() => auth.load('src/lib/bff-session.ts').bffSessionCoordinator.dispose())
  await assert.rejects(auth.state().initializeSessionCoordination(), error => error.status === 503)
  assert.equal(auth.state().authMode, 'BFF')
  assert.equal(auth.state().sessionStatus, 'unavailable')
  assert.equal(auth.state().bffSession, null)
  assert.equal(auth.state().token, null)

  const opening = deferred()
  const page = recoveryPage(auth.state(), { login: () => opening.promise })
  t.after(() => { opening.resolve(); page.unmount() })
  const requests = auth.calls.length, snapshot = authoritySnapshot(auth)
  let tree = page.render()
  const login = button(tree, '继续统一登录')
  assert.equal(login.props.disabled, false)
  assert.deepEqual(page.calls, { login: 0, reload: 0, select: [], logout: [] })
  assert.equal(auth.calls.length, requests)
  assert.equal(authoritySnapshot(auth), snapshot)
  login.props.onClick(); login.props.onClick()
  assert.deepEqual(page.calls, { login: 1, reload: 0, select: [], logout: [] })
  tree = page.render()
  assert.ok(text(tree).includes('正在打开统一登录'))
  assert.equal(auth.calls.length, requests, 'The view delegates a user login intent without retrying unavailable authority')
  assert.equal(authoritySnapshot(auth), snapshot)
  opening.resolve(); await settle()
  assert.equal(button(page.render(), '继续统一登录').props.disabled, false)
})

test('the unavailable login exception never exposes a private BFF route or an unconfirmed or LEGACY mode', () => {
  for (const { authMode, pathname } of [
    { authMode: 'BFF', pathname: '/system/users' },
    { authMode: 'BFF', pathname: '/dashboard' },
    { authMode: null, pathname: '/login' },
    { authMode: 'LEGACY', pathname: '/login' },
  ]) {
    const privateContent = `must-remain-private:${String(authMode)}:${pathname}`
    const tree = sessionGate({ authMode, sessionStatus: 'unavailable', coordinationError: 'Owned unavailable authority',
      sessionId: null, contextEpoch: 0 }, pathname)(privateContent)
    assert.equal(text(tree).includes(privateContent), false, `${String(authMode)} ${pathname} must retain the real session gate`)
    assert.equal(tree.type, 'main')
    assert.equal(tree.props.role, 'status')
    assert.ok(text(tree).includes('Owned unavailable authority'))
  }
})
