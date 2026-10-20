import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { createHash } from 'node:crypto'
import { StoresClient } from './old-business/stores/src/client.ts'
import { WorkOrdersClient } from './old-business/work-orders/src/client.mjs'
const setup = JSON.parse(process.env.PHASE31_CLIENTS)
const provenance=JSON.parse(readFileSync(new URL('./old-business-provenance.json',import.meta.url)))
for(const item of provenance.files)assert.equal(createHash('sha256').update(readFileSync(new URL('./old-business/'+item.file,import.meta.url))).digest('hex'),item.sha256)
function props(code) {
  return { ...setup[code].context, sdkVersion: '1.0.0', contractVersion: '2.0', basePath: '/micro/'+code,
    routePath: '/micro/'+code, contextEpoch: 1, token: setup[code].token,
    tokenExpiresAt: new Date(Date.now()+60000).toISOString(), theme: 'light',
    signal: new AbortController().signal, onNavigate() {}, onNotify() {} }
}
const stores = new StoresClient({ apiBase: setup.stores.origin+'/stores-api' })
console.log('PHASE31_CLIENT_STAGE STORES_CONTEXT')
assert.equal(stores.setContext(props('stores')), true)
console.log('PHASE31_CLIENT_STAGE STORES_WRITE')
const store = await stores.createStore({name:'Phase31 old client',businessStatus:'OPEN',businessContact:''})
assert.match(store.id,/^[0-9a-f-]{36}$/)
assert.equal((await stores.updateStore(store.id,{version:store.version,name:'Phase31 compatible'})).version,2)
assert.equal((await stores.listStores()).total,1)
console.log('PHASE31_CLIENT_STAGE WORK_CONTEXT')
const work = new WorkOrdersClient({ apiBase: setup['work-orders'].origin+'/work-orders-api' })
work.update(props('work-orders'))
console.log('PHASE31_CLIENT_STAGE WORK_WRITE')
const ticket = await work.create({title:'Phase31 old client',description:'compatibility',priority:'NORMAL'},'phase31-old-client')
assert.match(ticket.id,/^[0-9a-f-]{36}$/)
assert(ticket.number.length>0)
assert.equal((await work.create({title:'Phase31 old client',description:'compatibility',priority:'NORMAL'},'phase31-old-client')).id,ticket.id)
assert.equal((await work.updateTicket(ticket.id,{version:ticket.version,title:'Phase31 compatible'})).version,2)
assert.equal((await work.list({})).total,1)
stores.invalidate();work.stop()
console.log('PHASE31_OLD_BUSINESS PASS stores_create_update_list work_create_replay_update_list')
