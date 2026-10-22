import assert from 'node:assert/strict'
import test from 'node:test'
import { componentFixture, elements, button } from './helpers/component-fixture.mjs'

function fixture() {
  const state = { token: 'platform-session', contextEpoch: 1 }
  const store = selector => selector(state)
  store.getState = () => state
  let resolveInfo
  const info = new Promise(resolve => { resolveInfo = resolve })
  const decisions = []
  const f = componentFixture('src/pages/device/DevicePage.tsx', ['DevicePage'], {
    '@/stores/auth': { useAuthStore: store },
    'react-router-dom': { Navigate: 'Navigate', useSearchParams: () => [new URLSearchParams('user_code=AAAA-AAAA')] },
    '@/hooks/use-document-title': { useDocumentTitle() {} },
    '@/hooks/use-platform-branding': { usePlatformBranding: () => ({}) },
    '@/lib/format': { SCOPE_LABELS: {}, formatRelative: value => value },
    '@/api/public': { deviceApi: { info: () => info, decide: async (...args) => { decisions.push(args) } } },
  })
  return { ...f, state, decisions, resolveInfo: () => resolveInfo({ clientId: 'client-a', clientName: 'Client A', status: 'PENDING', scope: 'read', expiresAt: '2099-01-01' }) }
}

test('device approval uses the exact code whose client and scopes were displayed', async () => {
  const f = fixture()
  let tree = f.render('DevicePage')
  elements(tree, item => item.type === 'form')[0].props.onSubmit({ preventDefault() {} })
  await Promise.resolve()
  // Editing the input while A is loading must not retarget A's consent to B.
  elements(tree, item => item.type === 'Input')[0].props.onChange({ target: { value: 'BBBB-BBBB' } })
  f.render('DevicePage')
  f.resolveInfo()
  await f.mutations[0].promise
  tree = f.render('DevicePage')
  assert.match(JSON.stringify(tree), /AAAA-AAAA/)
  button(tree, '批准授权').props.onClick()
  await f.mutations[1].promise
  assert.deepEqual(f.decisions, [['AAAA-AAAA', true]])
})

test('changing login context immediately retires a previously displayed device consent', async () => {
  const f = fixture()
  let tree = f.render('DevicePage')
  elements(tree, item => item.type === 'form')[0].props.onSubmit({ preventDefault() {} })
  f.resolveInfo()
  await f.mutations[0].promise
  tree = f.render('DevicePage')
  assert.ok(button(tree, '批准授权'))
  f.state.contextEpoch++
  tree = f.render('DevicePage', {}, false)
  assert.equal(button(tree, '批准授权'), undefined)
  assert.equal(f.decisions.length, 0)
})
