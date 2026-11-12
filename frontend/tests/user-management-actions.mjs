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

test('closed membership writes disable the single merged creation action', () => {
  const element = UserManagementActions({ memberType: 'EMPLOYEE', createMembershipBlockedReason: '员工管理尚未启用',
    onAddMembership() {} })
  const html = renderToStaticMarkup(element)
  const buttons = [...html.matchAll(/<button\b[^>]*>[\s\S]*?<\/button>/g)].map(match => match[0])
  assert.equal(buttons.length, 1)
  assert.match(buttons[0], /新建员工/)
  assert.match(buttons[0], /disabled=/)
  assert.match(buttons[0], /title="员工管理尚未启用"/)
  assert.doesNotMatch(html, /新增账号/)
})

test('customer creation opens one combined flow', () => {
  const events = []
  const element = UserManagementActions({ memberType: 'CUSTOMER', createMembershipBlockedReason: null,
    onAddMembership: () => events.push('membership') })
  const membership = element.props.children
  membership.props.onClick()
  assert.deepEqual(events, ['membership'])
  const html = renderToStaticMarkup(element)
  assert.match(html, /新建客户/)
  assert.doesNotMatch(html, /disabled=/)
})

const policyOutput = await build({ entryPoints: [fileURLToPath(new URL('../src/pages/system/users/new-member-policy.ts', import.meta.url))], bundle: true, write: false, platform: 'node', format: 'cjs', packages: 'external', tsconfig: fileURLToPath(new URL('../tsconfig.json', import.meta.url)) })
const policy = { exports: {} }
new Function('require', 'module', 'exports', policyOutput.outputFiles[0].text)(createRequire(import.meta.url), policy, policy.exports)
const { newMemberPayload } = policy.exports
const organization = '019a0000-0000-7000-8000-000000000001'
const account = { username: 'member', password: 'Member-Password9!', realName: '测试成员', phone: '138 0000-0001', email: '' }

test('employee and customer creation require a phone and send the requested identity', () => {
  for (const type of ['EMPLOYEE', 'CUSTOMER']) {
    assert.throws(() => newMemberPayload({...account,phone:''}, organization, type, '', ''), /电话必填/)
    assert.throws(() => newMemberPayload({...account,phone:' () - '}, organization, type, '', ''), /电话必填/)
    const payload = newMemberPayload(account, organization, type, '岗位', '')
    assert.equal(payload.membership.memberType, type)
    assert.equal(payload.membership.organizationId, organization)
    assert.equal(payload.membership.position, type === 'EMPLOYEE' ? '岗位' : '')
    assert.ok(payload.membership.reason)
    assert.equal(payload.email, '')
  }
  assert.throws(() => newMemberPayload({...account,email:'bad-email'},organization,'EMPLOYEE','',''), /邮箱格式/)
  const automatic = newMemberPayload({username:'',password:'',realName:'',email:'',phone:account.phone},organization,'CUSTOMER','','')
  assert.equal(automatic.phone,account.phone)
  assert.equal(automatic.username,'')
})
