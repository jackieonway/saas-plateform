import assert from 'node:assert/strict'
import test from 'node:test'
import { componentFixture, success, button } from './helpers/component-fixture.mjs'

function deferred() {
  let resolve
  const promise = new Promise(done => { resolve = done })
  return { promise, resolve }
}

function fixture(t, overrides = {}) {
  const state = { token: 'session', contextEpoch: 1, contextSwitching: false, identity: 'user:tenant:org-a' }
  let requestId = 'request-a'
  const store = selector => selector(state)
  store.getState = () => state
  const decisions = []
  const originalWindow = globalThis.window
  globalThis.window = { location: { href: '/consent' } }
  t.after(() => { globalThis.window = originalWindow })
  const info = { client: { clientName: 'Client A' }, scope: 'read', redirectUri: 'https://client.example/callback' }
  const f = componentFixture('src/pages/consent/ConsentPage.tsx', ['ConsentPage'], {
    '@/stores/auth': { useAuthStore: store },
    'react-router-dom': { Navigate: 'Navigate', useSearchParams: () => [new URLSearchParams({ request_id: requestId })] },
    '@/hooks/use-document-title': { useDocumentTitle() {} },
    '@/hooks/use-platform-branding': { usePlatformBranding: () => ({}) },
    '@/lib/format': { SCOPE_LABELS: {} },
    '@/lib/auth-queries': {
      captureAuthSession: () => ({ epoch: state.contextEpoch, token: state.token, identity: state.identity, switching: state.contextSwitching }),
      isCurrentAuthSession: captured => !!state.token && !state.contextSwitching && !captured.switching &&
        captured.epoch === state.contextEpoch && captured.identity === state.identity,
    },
    '@/api/public': { consentApi: {
      info: overrides.info ?? (async () => info),
      decide: async (...args) => { decisions.push(args); return overrides.decide ? overrides.decide(...args) : { redirectTo: 'https://client.example/callback?code=approved' } },
    } },
  })
  return { ...f, state, decisions, info, setRequest: value => { requestId = value },
    async ready() {
      f.render('ConsentPage')
      f.queries.set('consent', success(await f.queryOptions.get('consent').queryFn()))
      return f.render('ConsentPage')
    },
  }
}

test('consent decisions bind the reviewed request and synchronously reject repeated clicks', async t => {
  const f = fixture(t)
  const tree = await f.ready()
  const approve = button(tree, '同意授权')
  approve.props.onClick()
  approve.props.onClick()
  await f.mutations[0].promise
  assert.deepEqual(f.decisions, [['request-a', true]])
  assert.equal(globalThis.window.location.href, 'https://client.example/callback?code=approved')
  assert.deepEqual(f.queryOptions.get('consent').queryKey, ['consent', 1, 'user:tenant:org-a', 'request-a'])
})

test('context switching hides consent and rejects callbacks retained from the old view', async t => {
  const f = fixture(t)
  const tree = await f.ready()
  f.state.contextSwitching = true
  const switched = f.render('ConsentPage')
  assert.equal(button(switched, '同意授权'), undefined)
  assert.equal(f.queryOptions.get('consent').enabled, false)
  button(tree, '同意授权').props.onClick()
  assert.deepEqual(f.decisions, [])
})

test('an info response arriving after the login context changed is rejected', async t => {
  const pending = deferred()
  const f = fixture(t, { info: () => pending.promise })
  f.render('ConsentPage')
  const read = f.queryOptions.get('consent').queryFn()
  f.state.contextEpoch++
  pending.resolve(f.info)
  await assert.rejects(read, error => error.name === 'AbortError')
})

test('a retired request cannot redirect the newly opened consent page', async t => {
  const pending = deferred()
  const f = fixture(t, { decide: () => pending.promise })
  const tree = await f.ready()
  button(tree, '同意授权').props.onClick()
  await Promise.resolve()
  f.setRequest('request-b')
  const next = f.render('ConsentPage')
  assert.equal(button(next, '同意授权'), undefined)
  pending.resolve({ redirectTo: 'https://client.example/stale' })
  await f.mutations[0].promise
  assert.equal(globalThis.window.location.href, '/consent')
})

test('cached consent information is retired before a new context can approve it', async t => {
  const f = fixture(t)
  await f.ready()
  f.state.contextEpoch++
  f.state.identity = 'user:tenant:org-b'
  const tree = f.render('ConsentPage', {}, false)
  assert.equal(button(tree, '同意授权'), undefined)
  assert.deepEqual(f.queryOptions.get('consent').queryKey, ['consent', 2, 'user:tenant:org-b', 'request-a'])
})

test('a completed consent response cannot redirect after the page unmounts', async t => {
  const pending = deferred()
  const f = fixture(t, { decide: () => pending.promise })
  const tree = await f.ready()
  button(tree, '同意授权').props.onClick()
  await Promise.resolve()
  f.unmount()
  pending.resolve({ redirectTo: 'https://client.example/stale' })
  await f.mutations[0].promise
  assert.equal(globalThis.window.location.href, '/consent')
})
