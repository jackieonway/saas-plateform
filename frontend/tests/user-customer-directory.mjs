import assert from 'node:assert/strict'
import test from 'node:test'
import { componentFixture, success, elements } from './helpers/component-fixture.mjs'
import * as model from '../src/pages/system/users/membership-model.ts'

const id = n => `019c0000-0000-7000-8000-${String(n).padStart(12,'0')}`
const capabilities = {lifecycleEnabled:true,customerMembershipsEnabled:true,dualMembershipsEnabled:true}
const member = type => ({id:id(type==='EMPLOYEE'?3:4),userId:id(1),organizationId:id(2),memberType:type,status:'ENABLED',identityVersion:1,departmentId:null,position:'',updatedAt:'2026-10-27T00:00:00Z'})

test('users page has one employee information directory without customer or global-account tabs', () => {
  const f=componentFixture('src/pages/system/users/UsersPage.tsx',['UsersPage'],{
    '@/stores/auth':{useAuthStore:selector=>selector({contextKey:'scope'})},
    './MembershipActionDialog':{MembershipActionDialog:'MembershipActionDialog'},
    './membership-model':model,
  })
  const tree=f.render('UsersPage')
  assert.equal(tree.type.name,'UserDirectory')
  assert.equal(tree.props.memberType,'EMPLOYEE')
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
  const submit=elements(tree,n=>n.type==='Button'&&n.props.children===`添加${type==='EMPLOYEE'?'员工':'客户'}`)[0]
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
  assert.equal(elements(tree,n=>n.type==='Button'&&n.props.children==='添加员工')[0].props.disabled,true)
})
