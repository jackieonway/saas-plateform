import assert from 'node:assert/strict'
import test from 'node:test'
import { componentFixture, elements, button, success } from './helpers/component-fixture.mjs'

function fixture(write = true) {
  const calls = []
  let current = true
  const api = Object.fromEntries(['devices','users','revoke','revokeAll','recover'].map(method => [method, async id => { calls.push([method,id]); return {} }]))
  const permissions = write ? ['system:session','system:session:revoke'] : ['system:session']
  const f = componentFixture('src/pages/system/sessions/SessionsPage.tsx', ['SessionWorkspace'], {
    '@/api/sessions': { sessionsApi: api },
    '@/hooks/use-auth-data': { usePermissionsQuery: () => success({permissions}) },
    '@/lib/auth-queries': { isCurrentAuthSession: () => current },
    '@/lib/format': { useDebounced: value => value, formatDateTime: value => value },
  })
  f.queries.set('admin-sessions',success({records:[{id:'device-a',userId:'user-a',username:'alice',realName:'Alice',tenantName:'平台',organizationName:'总部',active:true,isCurrent:false,activeSessions:2,sessions:3}],total:1}))
  return { ...f, api, calls, permissions, stale() { current = false }, render() { return f.render('SessionWorkspace',{session:{namespace:'owned',epoch:1}}) } }
}
const confirm = tree => elements(tree, item => item.type === 'ConfirmDialog')[0]

test('device revocation requires confirmation and sends the session id',async()=>{
  const f=fixture();let tree=f.render()
  button(tree,'吊销设备').props.onClick();tree=f.render()
  assert.equal(confirm(tree).props.open,true)
  assert.deepEqual(f.calls,[])
  confirm(tree).props.onConfirm();await f.mutations[0].promise
  assert.deepEqual(f.calls,[['revoke','device-a']])
})
test('user-wide revocation requires confirmation and uses the account id',async()=>{
  const f=fixture();let tree=f.render()
  elements(tree,item=>item.type==='Tabs')[0].props.onValueChange('users');tree=f.render()
  button(tree,'踢出全部').props.onClick();tree=f.render()
  confirm(tree).props.onConfirm();await f.mutations[0].promise
  assert.deepEqual(f.calls,[['revokeAll','user-a']])
})
test('read-only permission hides revocation controls',()=>{
  const tree=fixture(false).render()
  assert.equal(button(tree,'吊销设备'),undefined)
})
test('a confirmation saved before context change cannot dispatch',()=>{
  const f=fixture();let tree=f.render();button(tree,'吊销设备').props.onClick();tree=f.render()
  const old=confirm(tree);f.stale();old.props.onConfirm()
  assert.deepEqual(f.calls,[])
})
test('permission lost while confirmation is open cannot dispatch',()=>{
  const f=fixture();let tree=f.render();button(tree,'吊销设备').props.onClick();tree=f.render()
  f.permissions.splice(0);tree=f.render();confirm(tree).props.onConfirm()
  assert.deepEqual(f.calls,[])
})
test('unknown accepted operation blocks resubmission and recovers the exact operation',async()=>{
  const f=fixture();const operation='019a0000-0000-7000-8000-000000000001'
  f.api.revoke=async()=>{throw new Error(`账号操作结果未知，操作编号 ${operation}；请查询原操作结果`)}
  let tree=f.render();button(tree,'吊销设备').props.onClick();tree=f.render();confirm(tree).props.onConfirm();await f.mutations[0].promise
  tree=f.render();assert.equal(confirm(tree).props.loading,true)
  confirm(tree).props.onConfirm();assert.deepEqual(f.calls,[])
  button(tree,'恢复原操作').props.onClick();await f.mutations[1].promise
  assert.deepEqual(f.calls,[['recover',operation]])
})
test('filter changes reset pagination before a new query',()=>{
  const f=fixture();let tree=f.render();elements(tree,item=>item.type==='DataPagination')[0].props.onPageChange(4)
  tree=f.render();elements(tree,item=>item.type==='Input')[0].props.onChange({target:{value:'alice'}})
  f.render();const options=f.queryOptions.get('admin-sessions')
  assert.equal(options.queryKey.at(-1).page,1)
  assert.equal(options.queryKey.at(-1).keyword,'alice')
})
