import assert from 'node:assert/strict'
import test from 'node:test'
import { componentFixture, success, pending, failure, elements, button } from './helpers/component-fixture.mjs'

const role = { id: 'role-a', name: 'Role A', organizationId: 'org-a', organizationName: 'Org A' }
const user = { id: 'user-a', memberships: [{ organizationId: 'org-a', organizationName: 'Org A', memberType: 'EMPLOYEE', status: 'ENABLED' }] }
const props = value => ({ ...value, onClose() {}, onSaved() {} })
const userPageFixture = (names, dependencies = {}) => componentFixture('src/pages/system/users/UsersPage.tsx', names, {
  './MembershipDirectory': { MembershipDirectory: 'MembershipDirectory' },
  './MembershipActionDialog': { MembershipActionDialog: 'MembershipActionDialog' },
  './membership-model': { membershipWriteBlockedReason: () => null },
  ...dependencies,
})

test('role member replacement is blocked until existing assignments and candidates load successfully', async () => {
  const writes = []
  const f = componentFixture('src/pages/system/roles/RolesPage.tsx', ['RoleUsersDialog'], {
    '@/api/roles': { rolesApi: { saveUsers: (...args) => { writes.push(args) } } },
  })
  f.queries.set('users', success([{ id: 'existing' }]))
  let tree = f.render('RoleUsersDialog', props({ role }))
  assert.equal(button(tree, '保存成员').props.disabled, true)
  assert.throws(() => f.mutations[0].options.mutationFn(), /加载/)
  f.queries.set('role-users', failure())
  tree = f.render('RoleUsersDialog', props({ role }))
  assert.equal(button(tree, '保存成员').props.disabled, true)
  f.queries.set('role-users', success([{ userId: 'existing' }]))
  tree = f.render('RoleUsersDialog', props({ role }))
  assert.equal(button(tree, '保存成员').props.disabled, false)
  await f.mutations[0].options.mutationFn()
  assert.deepEqual(writes, [['role-a', ['existing']]])
})

test('failed role menu reads display an error and cannot overwrite permissions', () => {
  const f = componentFixture('src/pages/system/roles/RolesPage.tsx', ['RoleMenusDialog'])
  f.queries.set('authorizations', success({ applications: [{ id: 'app-a', authorized: true }], menus: {} }))
  f.queries.set('role-menus', failure())
  const tree = f.render('RoleMenusDialog', props({ role }))
  assert.equal(button(tree, '保存权限').props.disabled, true)
  assert.equal(elements(tree, item => item.type === 'ErrorState').length, 1)
  assert.throws(() => f.mutations[0].options.mutationFn(), /加载/)
})

test('background assignment refreshes do not discard unsaved role member edits', () => {
  const f = componentFixture('src/pages/system/roles/RolesPage.tsx', ['RoleUsersDialog'])
  f.queries.set('role-users', success([{ userId: 'existing' }]))
  f.queries.set('users', success([{ id: 'existing', realName: 'Existing' }]))
  let tree = f.render('RoleUsersDialog', props({ role }))
  elements(tree, item => item.type === 'Checkbox')[0].props.onCheckedChange(false)
  f.queries.set('role-users', success([{ userId: 'existing' }]))
  tree = f.render('RoleUsersDialog', props({ role }))
  assert.equal(elements(tree, item => item.type === 'Checkbox')[0].props.checked, false)
})

test('user role replacement waits for every candidate query and preserves existing roles outside candidates', async () => {
  const writes = []
  const f = userPageFixture(['RoleAssignDialog'], {
    '@/api/users': { usersApi: { saveRoles: async (...args) => { writes.push(args) } } },
  })
  f.queries.set('user-roles', success([{ roleId: 'retained', organizationId: 'other-org' }]))
  let tree = f.render('RoleAssignDialog', props({ user }))
  assert.equal(button(tree, '保存角色分配').props.disabled, true)
  await button(tree, '保存角色分配').props.onClick()
  assert.equal(writes.length, 0)
  f.queries.set('roles', failure())
  tree = f.render('RoleAssignDialog', props({ user }))
  assert.equal(button(tree, '保存角色分配').props.disabled, true)
  assert.equal(elements(tree, item => item.type === 'ErrorState').length, 1)
  f.queries.set('roles', success([]))
  tree = f.render('RoleAssignDialog', props({ user }))
  assert.equal(button(tree, '保存角色分配').props.disabled, false)
  await button(tree, '保存角色分配').props.onClick()
  assert.deepEqual(writes, [['user-a', [{ roleId: 'retained', organizationId: 'other-org' }]]])
})

test('user role candidates load every page beyond the backend page limit', async () => {
  const pages = []
  const f = userPageFixture(['RoleAssignDialog'], {
    '@/api/roles': { rolesApi: { list: async ({ page }) => {
      pages.push(page)
      return { total: 201, records: Array.from({ length: page === 1 ? 200 : 1 }, (_, index) => ({ id: `role-${(page - 1) * 200 + index}` })) }
    } } },
  })
  f.render('RoleAssignDialog', props({ user }))
  const records = await f.queryOptions.get('roles').queryFn()
  assert.equal(records.length, 201)
  assert.deepEqual(pages, [1, 2])
})

test('membership replacement cannot submit before cached assignments initialize or after a read error', async () => {
  const writes = []
  const f = userPageFixture(['MembershipDialog'], {
    '@/api/users': { usersApi: { saveMemberships: async (...args) => { writes.push(args) } } },
  })
  f.queries.set('user-memberships', success([]))
  let tree = f.render('MembershipDialog', props({ user, allOrgTree: [] }), false)
  assert.equal(button(tree, '保存成员关系').props.disabled, true)
  await button(tree, '保存成员关系').props.onClick()
  assert.equal(writes.length, 0)
  f.queries.set('user-memberships', failure())
  tree = f.render('MembershipDialog', props({ user, allOrgTree: [] }))
  assert.equal(button(tree, '保存成员关系').props.disabled, true)
})

test('employee role candidates exclude customer and disabled employee organization projections', async () => {
  const seen = []
  const f = userPageFixture(['RoleAssignDialog'], {
    '@/api/roles': { rolesApi: { list: async query => { seen.push(query.organizationId); return { total: 0, records: [] } } } },
  })
  f.queries.set('user-roles', success([]))
  f.queries.set('roles', success([]))
  f.render('RoleAssignDialog', props({ user: { ...user, memberships: [
    { organizationId: 'employee-org', memberType: 'EMPLOYEE', status: 'ENABLED' },
    { organizationId: 'employee-org', memberType: 'CUSTOMER', status: 'ENABLED' },
    { organizationId: 'customer-only-org', memberType: 'CUSTOMER', status: 'ENABLED' },
    { organizationId: 'disabled-org', memberType: 'EMPLOYEE', status: 'DISABLED' },
  ] } }))
  await f.queryOptions.get('roles').queryFn()
  assert.deepEqual(seen, ['employee-org'])
})

test('legacy employee bulk refuses an unproven or customer response before destructive replacement', async () => {
  for (const memberType of ['CUSTOMER', undefined]) {
    const f = userPageFixture(['MembershipDialog'], {
      '@/api/users': { usersApi: { memberships: async () => [{ organizationId: 'org-a', departmentId: null, position: '', memberType }] } },
    })
    f.render('MembershipDialog', props({ user, allOrgTree: [] }))
    await assert.rejects(f.queryOptions.get('user-memberships').queryFn(), /未确认的身份/)
  }
})

test('pending confirmation ignores duplicate destructive clicks', () => {
  let calls = 0
  const f = componentFixture('src/components/ConfirmDialog.tsx', ['ConfirmDialog'], {
    '@/components/ui/button': { buttonVariants: () => '' },
  })
  const tree = f.render('ConfirmDialog', { open: true, loading: true, onOpenChange() {}, onConfirm() { calls++ } })
  const action = elements(tree, item => item.type === 'AlertDialogAction')[0]
  assert.equal(action.props.disabled, true)
  action.props.onClick({ preventDefault() {} })
  assert.equal(calls, 0)
})

test('pagination recovers a deleted last page without reacting to loading or placeholder totals', () => {
  const changes = []
  const f = componentFixture('src/hooks/use-page-recovery.ts', ['usePageRecovery'])
  // Call the hook through a component so effect dependencies and rerenders follow React ordering.
  f.exports.Page = ({ query }) => f.exports.usePageRecovery(3, 10, query, value => changes.push(value))
  f.render('Page', { query: pending() })
  f.render('Page', { query: { ...success({ total: 0 }), isPlaceholderData: true } })
  assert.deepEqual(changes, [])
  f.render('Page', { query: success({ total: 20 }) })
  assert.deepEqual(changes, [2])
})
