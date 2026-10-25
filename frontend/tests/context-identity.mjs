import assert from 'node:assert/strict'
import test from 'node:test'
import { canSelectConsoleMembership, contextIdentityFromResponse, contextIdentityKey, isContextIdentity,
  membershipSelectionDisabledReason, membershipSelectionKey, requireContextIdentity, sameContextIdentity } from '../src/lib/context-identity.ts'

const id = number => `01962345-6789-7abc-8def-${String(number).padStart(12, '0')}`
const proof = (patch = {}) => ({ userId: id(1), tenantId: id(2), organizationId: id(3), membershipId: id(4),
  memberType: 'EMPLOYEE', identityVersion: '1', accountVersion: '0', ...patch })
const response = (identity = proof()) => ({ user: { id: identity.userId }, tenant: { id: identity.tenantId },
  organization: { id: identity.organizationId }, membershipId: identity.membershipId, memberType: identity.memberType,
  identityVersion: identity.identityVersion, accountVersion: identity.accountVersion })
const member = (patch = {}) => ({ ...proof(), status: 'ENABLED', consoleSelectable: true, selectionDisabledReason: null, ...patch })

test('only complete authenticated server proof can construct a context identity', () => {
  assert.deepEqual(contextIdentityFromResponse(response()), proof())
  assert.equal(Object.isFrozen(contextIdentityFromResponse(response())), true)
  assert.equal(contextIdentityFromResponse({ user: { id: id(1) }, tenant: { id: id(2) }, organization: { id: id(3) } }), null)
  for (const field of ['membershipId', 'memberType', 'identityVersion', 'accountVersion']) {
    const partial = response(); delete partial[field]
    assert.equal(contextIdentityFromResponse(partial), null, field)
  }
  for (const field of ['userId', 'tenantId', 'organizationId', 'membershipId']) {
    assert.equal(isContextIdentity(proof({ [field]: '11111111-2222-4333-8444-555555555555' })), false, field)
  }
  assert.throws(() => requireContextIdentity(response(proof({ memberType: 'CUSTOMER' }))), /员工成员证明/)
})

test('canonical decimal strings preserve the entire signed i64 version range', () => {
  assert.equal(isContextIdentity(proof({ identityVersion: '9223372036854775807', accountVersion: '9223372036854775807' })), true)
  for (const value of [1, 1.5, null, undefined, '01', '+1', '1.0', '1e3', '-1', '9223372036854775808']) {
    assert.equal(isContextIdentity(proof({ identityVersion: value })), false, String(value))
    assert.equal(isContextIdentity(proof({ accountVersion: value })), false, String(value))
  }
  assert.equal(isContextIdentity(proof({ identityVersion: '0' })), false)
  assert.equal(isContextIdentity(proof({ accountVersion: '0' })), true)
})

test('same user and organization never merge concrete members, types or generations', () => {
  const initial = proof(), key = contextIdentityKey(initial)
  for (const patch of [{ userId: id(5) }, { tenantId: id(5) }, { organizationId: id(5) }, { membershipId: id(5) },
    { memberType: 'CUSTOMER' }, { identityVersion: '2' }, { accountVersion: '1' }]) {
    assert.notEqual(contextIdentityKey(proof(patch)), key)
    assert.equal(sameContextIdentity(initial, proof(patch)), false)
  }
  assert.equal(sameContextIdentity(initial, proof()), true)
  assert.equal(sameContextIdentity(null, null), false)
})

test('selection keys bind server member proof and console never selects customer or disabled rows', () => {
  assert.equal(canSelectConsoleMembership(member()), true)
  assert.equal(membershipSelectionKey(proof()), membershipSelectionKey(member()))
  assert.notEqual(membershipSelectionKey(member()), membershipSelectionKey(member({ membershipId: id(5), memberType: 'CUSTOMER' })))
  for (const patch of [{ memberType: 'CUSTOMER' }, { status: 'DISABLED' }, { consoleSelectable: false },
    { membershipId: undefined }, { identityVersion: '01' }]) assert.equal(canSelectConsoleMembership(member(patch)), false)
  assert.equal(membershipSelectionDisabledReason(member({ memberType: 'CUSTOMER' })), '客户身份请使用客户入口')
  assert.equal(membershipSelectionDisabledReason(member({ status: 'DISABLED' })), '该成员身份已停用')
})
