import assert from 'node:assert/strict'
import test from 'node:test'
import { componentFixture, success, elements, button } from './helpers/component-fixture.mjs'
import * as uuid from '../src/lib/uuid.ts'

const model = componentFixture('src/pages/system/users/membership-model.ts', [], { '@/lib/uuid': uuid }).exports

const id = n => `019c0000-0000-7000-8000-${String(n).padStart(12,'0')}`
const capabilities = {lifecycleEnabled:true,customerMembershipsEnabled:true,dualMembershipsEnabled:true}
const member = type => ({id:id(type==='EMPLOYEE'?3:4),userId:id(1),organizationId:id(2),memberType:type,status:'ENABLED',identityVersion:1,departmentId:null,position:'',updatedAt:'2026-10-27T00:00:00Z'})

test('users page has one employee management directory without customer or global-account tabs', () => {
  const f=componentFixture('src/pages/system/users/UsersPage.tsx',['UsersPage'],{
    '@/stores/auth':{useAuthStore:selector=>selector({contextKey:'scope'})},
    './MembershipActionDialog':{MembershipActionDialog:'MembershipActionDialog'},
    './membership-model':model,
  })
  const tree=f.render('UsersPage')
  assert.equal(tree.type.name,'UserDirectory')
  assert.equal(tree.props.memberType,'EMPLOYEE')
})

function directoryFixture() {
  const writes = [], messages = []
  const form = { control: {}, reset() {}, handleSubmit: callback => callback }
  const f = componentFixture('src/pages/system/users/UsersPage.tsx', ['UserDirectory'], {
    '@/stores/auth': { useAuthStore: selector => selector({ contextKey: 'scope', profile: { user: { id: id(9) } } }) },
    '@/hooks/use-authorized-paths': { useAuthorizedPaths: () => ({ access: () => ({ status: 'allowed' }) }) },
    '@/hooks/use-synced-keyword': { useSyncedKeyword: () => ['', () => {}] },
    '@/hooks/use-page-recovery': { usePageRecovery() {} },
    '@/hooks/use-table-density': { useTableDensity: () => ({ density: 'normal' }), DENSITY_BODY_CLASS: {} },
    '@/hooks/use-column-visibility': { useColumnVisibility: () => ({ visibleSet: new Set(), isVisible: () => true }) },
    '@/lib/format': { useDebounced: value => value, formatDateTime: () => '', GENDER_LABELS: {} },
    '@/lib/form': { zResolver: () => undefined },
    'react-hook-form': { useForm: () => form },
    '@/api/users': { usersApi: { update: async (...args) => writes.push(args) } },
    './MembershipActionDialog': { MembershipActionDialog: 'MembershipActionDialog' },
    './membership-model': model,
    sonner: { toast: { success: value => messages.push(value), error: value => messages.push(value) } },
  })
  f.queries.set('membership-capabilities', success(capabilities))
  f.queries.set('tenants', success([]))
  f.queries.set('users', success({ records: [{ id: id(1), username: 'editable', realName: 'Before', email: 'before@example.test', phone: '13800138000', status: 'ENABLED', memberships: ['EMPLOYEE', 'CUSTOMER'].map(type => ({ organizationId: id(2), organizationName: 'Org', memberType: type, status: 'ENABLED' })) }], total: 1 }))
  return { ...f, writes, messages }
}

for (const memberType of ['EMPLOYEE', 'CUSTOMER']) test(`${memberType} profiles are editable by default and cleared contacts are sent explicitly`, async () => {
  const f = directoryFixture()
  let tree = f.render('UserDirectory', { memberType })
  const edit = elements(tree, n => n.type === 'Button' && n.props['aria-label'] === '编辑')[0]
  assert(edit, 'The default production directory must expose profile editing')
  const employeeActions = elements(tree, n => n.type === 'Button' && ['员工成员批量维护', '分配角色'].includes(n.props['aria-label']))
  assert.equal(employeeActions.length, memberType === 'EMPLOYEE' ? 2 : 0)
  edit.props.onClick()
  tree = f.render('UserDirectory', { memberType })
  elements(tree, n => n.type === 'form' && n.props.id === 'user-form')[0].props.onSubmit({ realName: 'After', email: '', phone: '', gender: 'FEMALE' })
  await f.mutations[0].promise
  assert.deepEqual(f.writes, [[id(1), { realName: 'After', email: '', phone: '', gender: 'FEMALE' }]])
  assert.equal(f.messages.at(-1), '用户已更新')
  const header = elements(tree, n => n.type === 'PageHeader')[0]
  header.props.actions.props.onClick()
  tree = f.render('UserDirectory', { memberType })
  assert.equal(elements(tree, n => n.type === 'MembershipActionDialog')[0].props.action.memberType, memberType)
})

test('an explicitly read-only directory still hides account mutations', () => {
  const f = directoryFixture()
  const tree = f.render('UserDirectory', { memberType: 'EMPLOYEE', informationOnly: true })
  assert.equal(elements(tree, n => n.type === 'Button' && n.props['aria-label'] === '编辑').length, 0)
  assert.equal(elements(tree, n => n.type === 'Switch').length, 0)
})

for(const type of ['EMPLOYEE','CUSTOMER']) test(`adding ${type} binds a searched account even when the other relationship exists`, async () => {
  const writes=[]
  const f=componentFixture('src/pages/system/users/MembershipActionDialog.tsx',['MembershipActionDialog'],{
    '@/stores/auth':{useAuthStore:selector=>selector({identity:{membershipId:id(9)}})},
    '@/api/memberships':{membershipsApi:{create:async body=>writes.push(body)}},
    './MembershipAccountPicker':{MembershipAccountPicker:'MembershipAccountPicker'},
    './membership-model':model,
  })
  f.queries.set('membership-counterpart', {...success({records:[member(type==='EMPLOYEE'?'CUSTOMER':'EMPLOYEE')]}),isFetching:false})
  const props={action:{kind:'create',memberType:type},namespace:'scope',capabilities,organizations:[],onClose(){},onSaved(){}}
  let tree=f.render('MembershipActionDialog',props)
  elements(tree,n=>n.type==='MembershipAccountPicker')[0].props.onChange({id:id(1),username:'existing',realName:'Existing'})
  elements(tree,n=>n.type==='EntityTreePicker')[0].props.onSelect(id(2))
  tree=f.render('MembershipActionDialog',props)
  elements(tree,n=>n.type==='Textarea')[0].props.onChange({target:{value:'管理员核实后绑定'}})
  if(type==='CUSTOMER') {
    elements(tree,n=>n.type==='Checkbox')[0].props.onCheckedChange(true)
    elements(tree,n=>n.type==='Input'&&n.props.id==='member-binding-source')[0].props.onChange({target:{value:'已核实所属组织'}})
  }
  tree=f.render('MembershipActionDialog',props)
  const submit=button(tree,`添加${type==='EMPLOYEE'?'员工':'客户'}`)
  assert.equal(submit.props.disabled,false)
  assert.doesNotMatch(JSON.stringify(tree),/改用明确的第二身份流程/)
  await submit.props.onClick()
  assert.equal(writes.length,1)
  assert.equal(writes[0].userId,id(1))
  assert.equal(writes[0].memberType,type)
})

test('same-type existing relationship remains blocked instead of creating a duplicate', () => {
  const f=componentFixture('src/pages/system/users/MembershipActionDialog.tsx',['MembershipActionDialog'],{
    '@/stores/auth':{useAuthStore:selector=>selector({identity:{membershipId:id(9)}})},
    './MembershipAccountPicker':{MembershipAccountPicker:'MembershipAccountPicker'},
    './membership-model':model,
  })
  f.queries.set('membership-counterpart', {...success({records:[member('EMPLOYEE')]}),isFetching:false})
  const tree=f.render('MembershipActionDialog',{action:{kind:'create',memberType:'EMPLOYEE'},namespace:'scope',capabilities,organizations:[],onClose(){},onSaved(){}})
  assert.equal(button(tree,'添加员工').props.disabled,true)
})
