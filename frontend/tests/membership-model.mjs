import assert from 'node:assert/strict'
import test from 'node:test'
import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
import { fixture as sessionFixture, json } from './helpers/session-fixture.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const sourceRoot = process.env.MEMBERSHIP_SOURCE_ROOT ? path.resolve(root, process.env.MEMBERSHIP_SOURCE_ROOT) : root
const require = createRequire(path.join(root, 'package.json'))
const ts = require('typescript')
const cache = new Map()
function load(relative) {
  if (cache.has(relative)) return cache.get(relative)
  const source = fs.readFileSync(path.join(sourceRoot, relative), 'utf8')
  const output = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText
  const exports = {}; cache.set(relative, exports)
  vm.runInNewContext(`(function(require,exports){${output}\n})`, { URLSearchParams })(
    name => name.startsWith('@/') ? load(`src/${name.slice(2)}.ts`) : require(name), exports,
  )
  return exports
}
const model = load('src/pages/system/users/membership-model.ts')
const {
  parseMembershipCapabilities, parseMembershipRecord, parseMembershipPage,
  membershipWriteBlockedReason, currentEmployeeMutationBlockedReason,
  buildCreateMembership, buildMembershipStatusChange, buildMembershipDelete,
  buildIdentityTransition, legacyEmployeeMembershipItems, oppositeMemberType,
} = model
const id = number => `019d0000-0000-7000-8000-${String(number).padStart(12, '0')}`
const caps = () => ({ lifecycleEnabled: true, customerMembershipsEnabled: true, dualMembershipsEnabled: true })
const row = (memberType = 'EMPLOYEE', extra = {}) => ({
  id: id(memberType === 'EMPLOYEE' ? 3 : 4), userId: id(1), organizationId: id(2),
  memberType, status: 'ENABLED', identityVersion: 7, departmentId: null, position: '',
  updatedAt: '2026-10-24T12:00:00Z', ...extra,
})
const draft = (memberType = 'EMPLOYEE', extra = {}) => ({
  userId: id(1), organizationId: id(2), memberType, reason: '核实后开通组织身份',
  ...(memberType === 'CUSTOMER' ? { accountBindingConfirmed: true, bindingSource: '管理员核实账号归属' } : {}), ...extra,
})
const intent = (extra = {}) => ({
  mode: 'secondIdentity', memberType: 'CUSTOMER', reason: '核实后新增客户身份',
  target: null, accountBindingConfirmed: true, bindingSource: '管理员核实账号归属', ...extra,
})
const plain = value => JSON.parse(JSON.stringify(value))
const invalid = operation => assert.throws(operation, error => error.name === 'MembershipValidationError')
const fixture = handler => sessionFixture(handler, { sourceRoot })
const api = f => f.load('src/api/memberships.ts').membershipsApi

test('capabilities accept exactly three booleans and never infer enabled from missing/unknown values', () => {
  assert.deepEqual(plain(parseMembershipCapabilities(caps())), caps())
  for (const value of [null, {}, { ...caps(), lifecycleEnabled: 'true' }, { ...caps(), dualMembershipsEnabled: 1 },
    { ...caps(), productionEnabled: true }, { lifecycleEnabled: true, customerMembershipsEnabled: true }]) {
    invalid(() => parseMembershipCapabilities(value))
  }
  assert.match(membershipWriteBlockedReason(undefined, 'create', 'EMPLOYEE'), /尚未确认/)
  assert.match(membershipWriteBlockedReason({ ...caps(), lifecycleEnabled: undefined }, 'delete'), /能力无效/)
})

test('membership DTOs preserve exact identity and reject unsafe i64 numbers or employee metadata on customers', () => {
  assert.deepEqual(plain(parseMembershipRecord(row())), row())
  assert.deepEqual(plain(parseMembershipRecord(row('CUSTOMER'))), row('CUSTOMER'))
  for (const identityVersion of [0, -1, 1.5, NaN, Infinity, '7', Number.MAX_SAFE_INTEGER + 1]) {
    invalid(() => parseMembershipRecord(row('EMPLOYEE', { identityVersion })))
  }
  assert.equal(parseMembershipRecord(row('EMPLOYEE', { identityVersion: Number.MAX_SAFE_INTEGER })).identityVersion, Number.MAX_SAFE_INTEGER)
  for (const extra of [{ status: 'ACTIVE' }, { memberType: 'ADMIN' }, { id: id(3).replace('-7000-', '-4000-') },
    { userId: id(1).replace('-8000-', '-c000-') }, { updatedAt: 'not-a-date' }, { roles: [] }]) invalid(() => parseMembershipRecord(row('EMPLOYEE', extra)))
  invalid(() => parseMembershipRecord(row('CUSTOMER', { departmentId: id(8) })))
  invalid(() => parseMembershipRecord(row('CUSTOMER', { position: '经理' })))
})

test('typed pagination keeps same-user/org identities separate and rejects duplicate member IDs', () => {
  const value = { records: [row(), row('CUSTOMER')], total: 2, page: 1, size: 10 }
  assert.deepEqual(plain(parseMembershipPage(value)), value)
  for (const extra of [{ records: [row(), row()] }, { total: 1 }, { page: 0 }, { size: 201 }, { size: 1 }, { total: '2' }, { extra: true }]) {
    invalid(() => parseMembershipPage({ ...value, ...extra }))
  }
})

test('server gates independently close lifecycle, customer creation and both combination modes', () => {
  for (const action of ['create', 'status', 'delete', 'secondIdentity', 'replaceIdentity']) {
    assert.match(membershipWriteBlockedReason({ ...caps(), lifecycleEnabled: false }, action, 'EMPLOYEE'), /生命周期/)
  }
  for (const action of ['create', 'secondIdentity', 'replaceIdentity']) {
    assert.match(membershipWriteBlockedReason({ ...caps(), customerMembershipsEnabled: false }, action, 'CUSTOMER'), /客户/)
  }
  for (const action of ['secondIdentity', 'replaceIdentity']) {
    assert.match(membershipWriteBlockedReason({ ...caps(), dualMembershipsEnabled: false }, action, 'EMPLOYEE'), /身份组合/)
  }
  for (const action of ['status', 'delete']) assert.equal(membershipWriteBlockedReason({ ...caps(), customerMembershipsEnabled: false, dualMembershipsEnabled: false }, action, 'CUSTOMER'), null)
})

test('create binds an explicitly selected existing account and projects only the lifecycle request fields', () => {
  const body = buildCreateMembership(draft('EMPLOYEE', { departmentId: id(8), position: ' 工程师 ', status: 'DISABLED', identityVersion: 99, membershipId: id(7), roles: ['admin'] }), caps())
  assert.deepEqual(plain(body), {
    userId: id(1), organizationId: id(2), memberType: 'EMPLOYEE', reason: '核实后开通组织身份', departmentId: id(8), position: '工程师',
  })
  assert.deepEqual(plain(buildCreateMembership(draft('CUSTOMER'), caps())), draft('CUSTOMER'))
  invalid(() => buildCreateMembership(draft('EMPLOYEE', { userId: '' }), caps()))
  invalid(() => buildCreateMembership(draft('EMPLOYEE', { organizationId: 'organization-a' }), caps()))
  invalid(() => buildCreateMembership(draft(), null))
  invalid(() => buildCreateMembership(draft('CUSTOMER'), { ...caps(), customerMembershipsEnabled: false }))
})

test('reasons count Unicode characters and reject empty, overlong or control-containing values', () => {
  assert.equal(buildCreateMembership(draft('EMPLOYEE', { reason: ` ${'🧭'.repeat(500)} ` }), caps()).reason, '🧭'.repeat(500))
  for (const reason of ['', '   ', '离职\n重新入职', '原因\u0000', '原因\u0085', '🧭'.repeat(501), undefined]) {
    invalid(() => buildCreateMembership(draft('EMPLOYEE', { reason }), caps()))
  }
  invalid(() => buildMembershipDelete(row(), '', caps()))
  invalid(() => buildIdentityTransition(row(), intent({ reason: '' }), caps()))
})

test('customer binding requires a literal confirmation and bounded single-line evidence', () => {
  for (const extra of [{ accountBindingConfirmed: false }, { accountBindingConfirmed: 'true' }, { bindingSource: '' },
    { bindingSource: 'x'.repeat(201) }, { bindingSource: 'first\nsecond' }, { bindingSource: 'first\u2028second' }]) {
    invalid(() => buildCreateMembership(draft('CUSTOMER', extra), caps()))
    invalid(() => buildIdentityTransition(row(), intent(extra), caps()))
  }
  assert.equal(buildCreateMembership(draft('CUSTOMER', { bindingSource: '🧭'.repeat(200) }), caps()).bindingSource, '🧭'.repeat(200))
  for (const extra of [{ departmentId: id(8) }, { position: '销售' }]) invalid(() => buildCreateMembership(draft('CUSTOMER', extra), caps()))
  invalid(() => buildCreateMembership(draft('EMPLOYEE', { position: '岗'.repeat(51) }), caps()))
})

test('known existing identities prevent duplicate creation and preserve the dual gate for either type', () => {
  invalid(() => buildCreateMembership(draft('EMPLOYEE', { existingMemberships: [row()] }), caps()))
  invalid(() => buildCreateMembership(draft('CUSTOMER', { existingMemberships: [row()] }), { ...caps(), dualMembershipsEnabled: false }))
  invalid(() => buildCreateMembership(draft('EMPLOYEE', { existingMemberships: [row('CUSTOMER')] }), { ...caps(), dualMembershipsEnabled: false }))
  assert.equal(buildCreateMembership(draft('CUSTOMER', { existingMemberships: [row()] }), caps()).memberType, 'CUSTOMER')
})

test('status/delete intents bind the exact selected member version and guard only the current employee', () => {
  assert.deepEqual(plain(buildMembershipStatusChange(row('CUSTOMER'), 'DISABLED', '暂停客户服务', caps(), row().id)), {
    expectedIdentityVersion: 7, reason: '暂停客户服务', status: 'DISABLED',
  })
  assert.deepEqual(plain(buildMembershipDelete(row('CUSTOMER'), '解除误绑定', caps(), row().id)), { expectedIdentityVersion: 7, reason: '解除误绑定' })
  assert.match(currentEmployeeMutationBlockedReason(row(), row().id), /当前登录/)
  assert.equal(currentEmployeeMutationBlockedReason(row('CUSTOMER'), row().id), null)
  invalid(() => buildMembershipStatusChange(row(), 'DISABLED', '离职', caps(), row().id))
  invalid(() => buildMembershipDelete(row(), '离职', caps(), row().id))
  assert.equal(buildMembershipDelete(row('EMPLOYEE', { id: id(9), organizationId: id(10) }), '离职', caps(), row().id).expectedIdentityVersion, 7)
})

test('combination creation uses the opposite type without target version or employee department fields', () => {
  assert.equal(oppositeMemberType('EMPLOYEE'), 'CUSTOMER')
  assert.equal(oppositeMemberType('CUSTOMER'), 'EMPLOYEE')
  assert.deepEqual(plain(buildIdentityTransition(row(), intent(), caps(), row().id)), {
    expectedIdentityVersion: 7, memberType: 'CUSTOMER', reason: '核实后新增客户身份', accountBindingConfirmed: true, bindingSource: '管理员核实账号归属',
  })
  assert.deepEqual(plain(buildIdentityTransition(row('CUSTOMER'), intent({ memberType: 'EMPLOYEE' }), caps())), {
    expectedIdentityVersion: 7, memberType: 'EMPLOYEE', reason: '核实后新增客户身份',
  })
  invalid(() => buildIdentityTransition(row(), intent({ memberType: 'EMPLOYEE' }), caps()))
  invalid(() => buildIdentityTransition(row(), intent({ mode: 'replaceIdentity' }), caps(), row().id))
  for (const mode of ['secondIdentity', 'replaceIdentity']) invalid(() => buildIdentityTransition(row(), intent({ mode }), { ...caps(), dualMembershipsEnabled: false }))
})

test('an existing target requires explicit reuse, its own version and an explicit enable of disabled identities', () => {
  const target = row('CUSTOMER', { identityVersion: 12, status: 'DISABLED' })
  invalid(() => buildIdentityTransition(row(), intent({ target }), caps()))
  invalid(() => buildIdentityTransition(row(), intent({ target, reuseTargetConfirmed: true }), caps()))
  const body = buildIdentityTransition(row(), intent({ target, reuseTargetConfirmed: true, enableTarget: true }), caps())
  assert.equal(body.expectedIdentityVersion, 7)
  assert.equal(body.expectedTargetIdentityVersion, 12)
  assert.equal(body.enableTarget, true)
  const enabled = buildIdentityTransition(row(), intent({ target: row('CUSTOMER'), reuseTargetConfirmed: true, enableTarget: true }), caps())
  assert.equal(enabled.expectedTargetIdentityVersion, 7)
  assert.equal(Object.hasOwn(enabled, 'enableTarget'), false)
})

test('unknown, mismatched or absent transition targets fail without guessing or forwarding a target version', () => {
  for (const target of [undefined, row('CUSTOMER', { userId: id(15) }), row('CUSTOMER', { organizationId: id(15) }), row(), row('CUSTOMER', { id: row().id })]) {
    invalid(() => buildIdentityTransition(row(), intent({ target, reuseTargetConfirmed: true, enableTarget: true }), caps()))
  }
  invalid(() => buildIdentityTransition(row(), intent({ enableTarget: true }), caps()))
  invalid(() => buildIdentityTransition(row(), intent({ reuseTargetConfirmed: true }), caps()))
  const input = intent({ expectedTargetIdentityVersion: 99, departmentId: id(8), status: 'ENABLED' })
  const body = buildIdentityTransition(row(), input, caps())
  for (const key of ['expectedTargetIdentityVersion', 'departmentId', 'status', 'membershipId']) assert.equal(Object.hasOwn(body, key), false)
})

test('legacy bulk projection retains disabled employees and excludes customer identity/proof/status fields', () => {
  const employee = row('EMPLOYEE', { status: 'DISABLED', departmentId: id(8), position: '工程师' })
  assert.deepEqual(plain(legacyEmployeeMembershipItems([employee, row('CUSTOMER')])), [{ organizationId: id(2), departmentId: id(8), position: '工程师' }])
  assert.deepEqual(plain(legacyEmployeeMembershipItems([row('CUSTOMER')])), [])
  invalid(() => legacyEmployeeMembershipItems([employee, row('EMPLOYEE', { id: id(9) })]))
})

test('typed client reads use the capabilities/detail paths and only the seven documented list filters', async () => {
  const f = await fixture(async url => url === '/api/memberships/capabilities' ? json(200, caps())
    : url.includes('?') ? json(200, { records: [row('CUSTOMER', { status: 'DISABLED' })], total: 1, page: 2, size: 20 }) : json(200, row()))
  const signal = new AbortController().signal
  assert.deepEqual(plain(await api(f).capabilities(signal)), caps())
  assert.equal((await api(f).detail(id(3))).id, id(3))
  await api(f).list({ userId: id(1), organizationId: id(2), tenantId: id(8), memberType: 'CUSTOMER', status: 'DISABLED', page: 2, size: 20, keyword: 'must-not-forward' })
  assert.equal(f.calls[0].url, '/api/memberships/capabilities')
  assert.equal(f.calls[1].url, `/api/memberships/${id(3)}`)
  const params = new URL(f.calls[2].url, 'https://platform.test').searchParams
  assert.deepEqual(Object.fromEntries(params), { userId: id(1), organizationId: id(2), tenantId: id(8), memberType: 'CUSTOMER', status: 'DISABLED', page: '2', size: '20' })
  assert.equal(f.calls[0].options.headers.Authorization, 'Bearer expired-access')
})

test('typed writes preserve exact methods, version bodies and transition source identity', async () => {
  const f = await fixture(async (url, config) => {
    if (config.method === 'DELETE') return json(200, { id: id(3), deleted: true, identityVersion: 8 })
    if (url.endsWith('/status')) return json(200, row('EMPLOYEE', { status: 'DISABLED', identityVersion: 8 }))
    if (url.endsWith('-identity')) return json(200, { ...row('CUSTOMER'), sourceMembershipId: id(3) })
    return json(200, row())
  })
  await api(f).create(buildCreateMembership(draft(), caps()))
  await api(f).changeStatus(id(3), buildMembershipStatusChange(row(), 'DISABLED', '离职', caps()))
  await api(f).remove(id(3), buildMembershipDelete(row(), '解除误绑定', caps()))
  const transition = buildIdentityTransition(row(), intent(), caps())
  await api(f).secondIdentity(id(3), transition)
  await api(f).replaceIdentity(id(3), transition)
  assert.deepEqual(f.calls.map(call => [call.options.method, call.url]), [
    ['POST', '/api/memberships'], ['PUT', `/api/memberships/${id(3)}/status`],
    ['DELETE', `/api/memberships/${id(3)}`], ['POST', `/api/memberships/${id(3)}/second-identity`], ['POST', `/api/memberships/${id(3)}/replace-identity`],
  ])
  assert.deepEqual(JSON.parse(f.calls[2].options.body), { expectedIdentityVersion: 7, reason: '解除误绑定' })
  assert.deepEqual(JSON.parse(f.calls[3].options.body), plain(transition))
})

test('malformed success payloads fail closed rather than becoming editable rows or optimistic success', async () => {
  const receipt = id(99)
  const f = await fixture(async () => {
    const response = json(200, { ...caps(), lifecycleEnabled: 'true' })
    response.headers.set('x-request-id', receipt)
    return response
  })
  await assert.rejects(api(f).capabilities(), error => error.code === 'PROTOCOL_ERROR' && error.status === 502 && error.requestId === receipt)
  const cases = [
    [() => row('EMPLOYEE', { id: id(8) }), client => client.detail(id(3))],
    [() => row('CUSTOMER'), client => client.create(draft())],
    [() => row('EMPLOYEE', { status: 'DISABLED' }), client => client.changeStatus(id(3), { status: 'DISABLED', expectedIdentityVersion: 7, reason: '离职' })],
    [() => ({ id: id(3), deleted: true, identityVersion: 7 }), client => client.remove(id(3), { expectedIdentityVersion: 7, reason: '离职' })],
    [() => ({ ...row('CUSTOMER'), sourceMembershipId: id(8) }), client => client.secondIdentity(id(3), buildIdentityTransition(row(), intent(), caps()))],
    [() => ({ records: [row('CUSTOMER')], total: 1, page: 1, size: 10 }), client => client.list({ memberType: 'EMPLOYEE' })],
  ]
  for (const [payload, operation] of cases) {
    const fixture = await sessionFixture(async () => json(200, payload()), { sourceRoot })
    await assert.rejects(operation(api(fixture)), error => error.code === 'PROTOCOL_ERROR' && error.status === 502)
  }
})

test('HTTP refusals preserve shared ApiError status and request ID without an automatic write retry', async () => {
  for (const status of [403, 404, 409, 503]) {
    const receipt = id(99)
    const f = await fixture(async () => {
      const result = json(status, { code: 'MEMBERSHIP_REFUSED', message: '请重新读取成员状态' })
      result.headers.set('x-request-id', receipt)
      return result
    })
    await assert.rejects(api(f).remove(id(3), { expectedIdentityVersion: 7, reason: '解除误绑定' }), error => error.status === status && error.requestId === receipt)
    assert.equal(f.calls.length, 1)
  }
})

test('invalid UUID or optimistic version is rejected before the shared transport can send a mutation', async () => {
  const f = await fixture(async () => json(200, row()))
  invalid(() => api(f).changeStatus('member-a', { expectedIdentityVersion: 7, status: 'DISABLED', reason: '离职' }))
  invalid(() => api(f).remove(id(3), { expectedIdentityVersion: Number.MAX_SAFE_INTEGER + 1, reason: '离职' }))
  invalid(() => api(f).create(draft('CUSTOMER', { accountBindingConfirmed: false })))
  assert.equal(f.calls.length, 0)
})
