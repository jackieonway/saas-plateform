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
    './UserManagementActions': { UserManagementActions: 'UserManagementActions' },
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
    './UserManagementActions': { UserManagementActions: 'UserManagementActions' },
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
  header.props.actions.props.onAddMembership()
  tree = f.render('UserDirectory', { memberType })
  assert.equal(elements(tree, n => n.type === 'MembershipActionDialog')[0].props.action.memberType, memberType)
})

test('an explicitly read-only directory still hides account mutations', () => {
  const f = directoryFixture()
  const tree = f.render('UserDirectory', { memberType: 'EMPLOYEE', informationOnly: true })
  assert.equal(elements(tree, n => n.type === 'Button' && n.props['aria-label'] === '编辑').length, 0)
  assert.equal(elements(tree, n => n.type === 'Switch').length, 0)
})


const creationPolicy=componentFixture('src/pages/system/users/new-member-policy.ts',[],{'@/lib/uuid':uuid}).exports
function creationFixture(api={}) {
  const writes=[],saved=[];const storage=new Map()
  Object.defineProperty(globalThis,'sessionStorage',{configurable:true,value:{getItem:k=>storage.get(k)??null,setItem:(k,v)=>storage.set(k,v),removeItem:k=>storage.delete(k)}})
  const f=componentFixture('src/pages/system/users/MembershipActionDialog.tsx',['MembershipActionDialog'],{
    '@/stores/auth':{useAuthStore:selector=>selector({identity:{membershipId:id(9)}})},
    '@/api/users':{usersApi:{create:async value=>{writes.push(value);return api.create?api.create(value):{id:id(1),username:'existing',accountReused:true}},recoverAccountOperation:async value=>{writes.push({recovery:value});return {ok:true}}}},
    './new-member-policy':creationPolicy,'./membership-model':model,
  })
  return {...f,writes,saved,storage}
}
for(const type of ['EMPLOYEE','CUSTOMER']) test(`creating ${type} delegates telephone matching without an account picker`,async()=>{
  const f=creationFixture()
  const props={action:{kind:'create',memberType:type},namespace:'scope',capabilities,organizations:[],onClose(){},onSaved(){f.saved.push(true)}}
  let tree=f.render('MembershipActionDialog',props)
  assert.equal(elements(tree,n=>n.type==='MembershipAccountPicker').length,0)
  assert.equal(button(tree,`新建${type==='EMPLOYEE'?'员工':'客户'}`).props.disabled,true)
  elements(tree,n=>n.type==='Input'&&n.props.id==='new-member-phone')[0].props.onChange({target:{value:'13800138000'}})
  elements(tree,n=>n.type==='EntityTreePicker')[0].props.onSelect(id(2))
  tree=f.render('MembershipActionDialog',props)
  const submit=button(tree,`新建${type==='EMPLOYEE'?'员工':'客户'}`)
  assert.equal(submit.props.disabled,false)
  await submit.props.onClick()
  assert.equal(f.writes.length,1)
  assert.equal(f.writes[0].phone,'13800138000')
  assert.equal(f.writes[0].username,'')
  assert.equal(f.writes[0].membership.memberType,type)
  assert.equal(f.saved.length,1)
})
test('contact conflict keeps the editor open and does not report success',async()=>{
  const f=creationFixture({create:async()=>{throw new Error('电话和邮箱属于不同账号')}})
  const props={action:{kind:'create',memberType:'EMPLOYEE'},namespace:'scope',capabilities,organizations:[],onClose(){},onSaved(){f.saved.push(true)}}
  let tree=f.render('MembershipActionDialog',props)
  elements(tree,n=>n.type==='Input'&&n.props.id==='new-member-phone')[0].props.onChange({target:{value:'13800138000'}})
  elements(tree,n=>n.type==='EntityTreePicker')[0].props.onSelect(id(2))
  tree=f.render('MembershipActionDialog',props);await button(tree,'新建员工').props.onClick()
  tree=f.render('MembershipActionDialog',props)
  assert.equal(f.saved.length,0)
  assert.ok(JSON.stringify(tree).includes('电话和邮箱属于不同账号'))
})
