import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { build } from 'esbuild'
import { renderToStaticMarkup } from 'react-dom/server'

const entry = fileURLToPath(new URL('../src/pages/system/users/UserManagementActions.tsx', import.meta.url))
const output = await build({ entryPoints: [entry], bundle: true, write: false, platform: 'node', format: 'cjs',
  packages: 'external', tsconfig: fileURLToPath(new URL('../tsconfig.json', import.meta.url)) })
const compiled = { exports: {} }
new Function('require', 'module', 'exports', output.outputFiles[0].text)(createRequire(import.meta.url), compiled, compiled.exports)
const { UserManagementActions } = compiled.exports

test('closed membership writes leave account creation available and show the reason', () => {
  const element = UserManagementActions({ memberType: 'EMPLOYEE', createMembershipBlockedReason: '员工管理尚未启用',
    onCreateAccount() {}, onAddMembership() {} })
  const html = renderToStaticMarkup(element)
  const buttons = [...html.matchAll(/<button\b[^>]*>[\s\S]*?<\/button>/g)].map(match => match[0])
  assert.equal(buttons.length, 2)
  assert.match(buttons[0], /新增账号/)
  assert.doesNotMatch(buttons[0], /disabled=/)
  assert.match(buttons[1], /添加员工/)
  assert.match(buttons[1], /disabled=/)
  assert.match(buttons[1], /title="员工管理尚未启用"/)
})

test('account creation and membership binding invoke separate actions', () => {
  const events = []
  const element = UserManagementActions({ memberType: 'CUSTOMER', createMembershipBlockedReason: null,
    onCreateAccount: () => events.push('account'), onAddMembership: () => events.push('membership') })
  const [account, membership] = element.props.children
  account.props.onClick()
  assert.deepEqual(events, ['account'])
  membership.props.onClick()
  assert.deepEqual(events, ['account', 'membership'])
  const html = renderToStaticMarkup(element)
  assert.match(html, /添加客户/)
  assert.doesNotMatch(html, /disabled=/)
})
