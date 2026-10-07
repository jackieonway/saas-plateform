import test from 'node:test'
import assert from 'node:assert/strict'
import {classifyPostgresFailure}from'../postgres.mjs'
import {withOperationStage}from'../pg-backup.mjs'
test('PostgreSQL diagnosis emits only fixed classes, validated SQLSTATE and process metadata',()=>{
  const secret='credential-private-value'
  const cases=[
    [`connection to server at private-host failed: Connection refused ${secret}`,'CONNECTION'],
    [`FATAL: password authentication failed for user "${secret}"`,'AUTHENTICATION'],
    [`ERROR: permission denied for table ${secret}`,'PERMISSION'],
    [`ERROR: 23514 ${secret}`,'CONSTRAINT'],
    [`ERROR: 57014 ${secret}`,'TIMEOUT_OR_CANCELLED'],
    [`ERROR: 22012 ${secret}`,'SQL_OR_SERVER'],
    [`ERROR: QUERY ${secret}`,'SQL_OR_SERVER'],
  ]
  for(const [raw,expected]of cases){const result=classifyPostgresFailure(raw,{exitCode:1});assert.equal(result.pgFailureClass,expected);assert.equal(JSON.stringify(result).includes(secret),false);assert.equal(JSON.stringify(result).includes('private-host'),false)}
  assert.equal(classifyPostgresFailure('ERROR: QUERY private').sqlState,undefined)
  assert.equal(classifyPostgresFailure('ERROR: 23514 private').sqlState,'23514')
  assert.deepEqual(classifyPostgresFailure('',{signal:'SIGTERM'}),{pgFailureClass:'PROCESS_SIGNAL',processSignal:'SIGTERM'})
  assert.deepEqual(classifyPostgresFailure('',{spawnCode:'ENOENT'}),{pgFailureClass:'TOOL_EXECUTION'})
  assert.equal(classifyPostgresFailure('',{signal:secret,exitCode:10000}).processSignal,undefined)
})
test('operation boundaries attach a closed stage enum without copying exception contents',async()=>{
  const error=new Error('private fixture value')
  await assert.rejects(withOperationStage('platform-auth-reset',async()=>{throw error}),caught=>caught===error && caught.operationStage==='platform-auth-reset')
  await assert.rejects(withOperationStage('unsafe-private-value',async()=>{}),{code:'INVALID_OPERATION_STAGE'})
})
