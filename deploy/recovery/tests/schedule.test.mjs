import test from 'node:test'
import assert from 'node:assert/strict'
import {runScheduled,parseSchedule}from'../scheduled-backup.mjs'
test('schedule stops every active writer before backup and preserves inactive service state',async()=>{
  const states=new Map([['platform.service','active'],['business.service','active'],['notifications.service','inactive']]),calls=[]
  const adapter={async state(unit){return states.get(unit)},async stop(unit){calls.push('stop:'+unit);states.set(unit,'inactive')},async start(unit){calls.push('start:'+unit);states.set(unit,'active')},async backup(){assert.deepEqual([...states.values()],['inactive','inactive','inactive']);calls.push('backup');return{event:'backup-complete',setId:'fixture'}}}
  const result=await runScheduled({envFile:'/secure/backup.env',units:[...states.keys()]},adapter)
  assert.deepEqual(calls,['stop:platform.service','stop:business.service','backup','start:platform.service','start:business.service'])
  assert.equal(states.get('notifications.service'),'inactive');assert.equal(result.restartedWriteServices,2);assert.equal(result.event,'scheduled-backup-complete')
})
test('backup and stop failures always restart previously active writers',async()=>{
  for(const failure of ['stop','backup']){
    let state='active',started=0,backupCalled=false
    const adapter={async state(){return state},async stop(){state='inactive';if(failure==='stop')throw Error('fixture')},async start(){started++;state='active'},async backup(){backupCalled=true;throw Error('fixture')}}
    await assert.rejects(runScheduled({envFile:'/secure/a',units:['platform.service']},adapter));assert.equal(started,1);assert.equal(state,'active');assert.equal(backupCalled,failure==='backup')
  }
})
test('schedule rejects command injection, duplicates and backup self-stop',()=>{
  for(const value of ['a.service;rm','a.service,a.service','saas-backup.service','a.service,,b.service'])assert.throws(()=>parseSchedule({BACKUP_ENV_FILE:'/secure/a',BACKUP_WRITE_UNITS:value}))
  assert.throws(()=>parseSchedule({BACKUP_ENV_FILE:'relative',BACKUP_WRITE_UNITS:'a.service'}))
})
